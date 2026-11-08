//! The real binary on the virtual display with sound and movies on: an ALSA config of ours gives it a real device path that ends in a file, so no speaker is reached and the mixed output can be read back. With no device at all it degrades and exits cleanly. Only on the virtual display, never the physical one.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn data_dir() -> PathBuf {
    std::env::var_os("OBLIVION_DATA_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../OblivionRecomp/Data"),
        PathBuf::from,
    )
}

fn scratch(name: &str) -> PathBuf {
    assert!(
        std::env::var_os("OBLIVION_VIRTUAL_DISPLAY_STATE").is_some(),
        "run this through tools/virtual_display.py exec"
    );
    let folder = std::env::temp_dir().join(format!("recomp_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

fn run(arguments: &[&str], alsa: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_OblivionRecompBevy"));
    command
        .env("OBLIVION_DATA_DIR", data_dir())
        .arg("--ini")
        .arg(fixtures_root().join("config/Oblivion.ini"))
        .args(["--seed", "1"])
        .args(arguments);
    if let Some(config) = alsa {
        command.env("ALSA_CONFIG_PATH", config);
    }
    command.output().unwrap()
}

// An ALSA device whose output runs into a reader of ours: a `null` device takes samples as fast as they come, gigabytes a second, so the reader drops the silence before the first sound and keeps `bytes` from there. `null` has no clock, so this checks samples, not their timing. A FIFO, because ALSA's `| command` form waits for its command at close. The reader holds both ends, so the FIFO never ends under it.
struct CaptureDevice {
    config: PathBuf,
    reader: Child,
}

impl CaptureDevice {
    fn new(folder: &Path, capture: &Path, bytes: u32) -> Self {
        let fifo = folder.join("device.fifo");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let script = format!(
            "import os\nfifo = os.open({fifo:?}, os.O_RDWR)\nout = open({capture:?}, 'wb')\nleft = {bytes}\nstarted = False\nwhile True:\n    b = os.read(fifo, 65536)\n    if not started:\n        b = b.lstrip(bytes(1))\n        if not b:\n            continue\n        started = True\n    if left > 0:\n        out.write(b[:left])\n        out.flush()\n        left -= len(b)\n"
        );
        let reader = Command::new("python3")
            .args(["-c", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let config = folder.join("asound.conf");
        std::fs::write(
            &config,
            format!(
                "pcm.!default {{ type plug; slave.pcm \"cap\" }}\npcm.cap {{ type file; slave.pcm \"null\"; file \"{}\"; format \"raw\" }}\npcm.null {{ type null }}\n",
                fifo.display()
            ),
        )
        .unwrap();
        Self { config, reader }
    }
}

impl Drop for CaptureDevice {
    fn drop(&mut self) {
        let _ = self.reader.kill();
        let _ = self.reader.wait();
    }
}

fn require_success(output: &Output) -> String {
    // `logging::emit` writes to stderr, and the phase lines to stdout.
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    text
}

/// Everything at once: the intro thread during the load, the logo, Map loop's frames and the title music through a real device path, in frames of the real app.
#[test]
fn the_app_plays_the_menu_s_movie_and_music() {
    let folder = scratch("av");
    let capture = folder.join("capture.raw");
    let device = CaptureDevice::new(&folder, &capture, 16_000_000);
    let frames = folder.join("frames");
    std::fs::create_dir_all(&frames).unwrap();
    let output = run(
        &[
            "--exit-after-frames",
            "600",
            "--dump-movie-frames",
            frames.to_str().unwrap(),
        ],
        Some(&device.config),
    );
    let stdout = require_success(&output);
    assert!(
        stdout.contains("Phase-06: Bink sound 1, intro thread started"),
        "{stdout}"
    );
    assert!(
        stdout.contains("MainMenu media: Oblivion iv logo.bik played, Map loop.bik playing"),
        "{stdout}"
    );
    assert!(stdout.contains("Audio start"), "{stdout}");
    assert!(
        stdout.contains("tes4title") || stdout.contains("TES4Title"),
        "{stdout}"
    );
    assert!(!stdout.contains("Audio device unavailable"), "{stdout}");
    assert!(stdout.contains("audio device Open"), "{stdout}");
    let written: Vec<_> = std::fs::read_dir(&frames)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        written.iter().any(|name| name.ends_with("1280x768.ppm")),
        "{written:?}"
    );
    // Interleaved 32-bit floats, as cpal picks from the `plug`. The title's peak is under 1.0, at 006AA209's gain of (0.6 * master 1.0) squared: 0.36, and the music must reach it.
    let mixed = std::fs::read(&capture).unwrap();
    assert!(mixed.len() > 1_000_000, "{} bytes", mixed.len());
    let peak = mixed
        .as_chunks::<4>()
        .0
        .iter()
        .map(|sample| f32::from_le_bytes(*sample).abs())
        .fold(0.0, f32::max);
    assert!((0.05..=0.361).contains(&peak), "peak {peak}");
    if let Some(keep) = std::env::var_os("RECOMP_KEEP_AV") {
        let keep = PathBuf::from(keep);
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::copy(&capture, keep.join("capture.raw")).unwrap();
        std::fs::write(keep.join("stdout.txt"), &stdout).unwrap();
        for name in &written {
            std::fs::copy(frames.join(name), keep.join(name)).unwrap();
        }
    }
    let _ = std::fs::remove_dir_all(&folder);
}

/// No audio device at all: the same run degrades, logs it, and exits 0, and the game's own lines don't change.
#[test]
fn no_audio_device_degrades() {
    let folder = scratch("nodevice");
    let config = folder.join("asound.conf");
    std::fs::write(&config, "pcm.!default { type hw; card 99 }\n").unwrap();
    let output = run(&["--exit-after-frames", "60"], Some(&config));
    let stdout = require_success(&output);
    assert!(stdout.contains("audio device Absent"), "{stdout}");
    assert!(stdout.contains("Phase-06: Bink sound 1"), "{stdout}");
    assert!(
        stdout.contains("MainMenu media: Oblivion iv logo.bik played, Map loop.bik playing"),
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(&folder);
}

/// `--no-audio-device` opens nothing, on any machine, and every voice fails.
#[test]
fn the_flag_opens_no_device() {
    let folder = scratch("noflag");
    let capture = folder.join("capture.raw");
    let device = CaptureDevice::new(&folder, &capture, 1000);
    let output = run(
        &["--no-audio-device", "--exit-after-frames", "30"],
        Some(&device.config),
    );
    std::thread::sleep(std::time::Duration::from_millis(200));
    let stdout = require_success(&output);
    assert_eq!(
        std::fs::metadata(&capture).map_or(0, |file| file.len()),
        0,
        "the device took samples"
    );
    assert!(stdout.contains("audio device Disabled"), "{stdout}");
    let _ = std::fs::remove_dir_all(&folder);
}
