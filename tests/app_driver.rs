//! The real binary on the virtual display: the G3 lines against `fixtures/cpp-baseline`, with `--seed` and the pinned config, then the same lines through the Documents path with scratch folders. The C++ `app_startup` allowed 600 s. Only on the virtual display, never the physical one.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn data_dir() -> PathBuf {
    std::env::var_os("OBLIVION_DATA_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../OblivionRecomp/Data"),
        PathBuf::from,
    )
}

fn require_display() {
    assert!(
        std::env::var_os("OBLIVION_VIRTUAL_DISPLAY_STATE").is_some(),
        "run this through tools/virtual_display.py exec"
    );
}

fn app() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_OblivionRecompBevy"));
    command.env("OBLIVION_DATA_DIR", data_dir());
    command
}

fn phase_lines(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("[INFO] "))
        .filter(|line| line.starts_with("Phase-"))
        .map(str::to_owned)
        .collect()
}

fn baseline() -> Vec<String> {
    let bytes = std::fs::read(fixtures_root().join("cpp-baseline/LastTest-x64.log")).unwrap();
    let text: String = bytes.iter().map(|&byte| char::from(byte)).collect();
    let body = &text[text
        .find("Testing: app_startup\n")
        .or_else(|| text.find("Testing: app_startup\r\n"))
        .unwrap()..];
    body[..body.find("<end of output>").unwrap()]
        .lines()
        .filter_map(|line| line.trim_end().strip_prefix("[INFO] "))
        .filter(|line| line.starts_with("Phase-"))
        .map(str::to_owned)
        .collect()
}

// The declared tolerances, as `recomp_startup/tests/winmain_driver.rs` gives their reasons.
fn tolerant(line: &str) -> String {
    // The C++ stops at 11 of these models, and the original builds all 26 (`the_driver_builds_the_recorded_model_trees`).
    if line.starts_with("Phase-13e:") {
        return "Phase-13e: ~".to_owned();
    }
    let mut out = line.to_owned();
    for key in [
        "Bink sound ",
        "editor IDs ",
        "item scripts not executed ",
        "doors not registered ",
        "change records ",
        "view ",
    ] {
        if let Some(at) = out.find(key) {
            let start = at + key.len();
            let end = out[start..]
                .find([',', ' '])
                .map_or(out.len(), |length| start + length);
            out.replace_range(start..end, "~");
        }
    }
    out
}

fn assert_g3(output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ours: Vec<String> = phase_lines(output)
        .iter()
        .map(|line| tolerant(line))
        .collect();
    let wanted: Vec<String> = baseline().iter().map(|line| tolerant(line)).collect();
    assert_eq!(ours, wanted);
    assert!(stdout.contains("0 host stops"), "{stdout}");
}

#[test]
fn the_binary_prints_the_g3_lines() {
    require_display();
    let started = Instant::now();
    let output = app()
        .arg("--ini")
        .arg(fixtures_root().join("config/Oblivion.ini"))
        .args(["--seed", "1", "--exit-after-frames", "5"])
        .output()
        .unwrap();
    println!("the whole run, with the window: {:.2?}", started.elapsed());
    assert_g3(&output);
    // Seed 1's rolls, on the real binary too.
    let lines = phase_lines(&output);
    assert!(
        lines[6].contains("item scripts not executed 1343"),
        "{}",
        lines[6]
    );
    assert!(lines[9].contains("change records 5497"), "{}", lines[9]);
}

// `path_setup` for a real run, on scratch Documents and LocalAppData folders: never the user's. It reads them and writes nothing.
#[test]
fn the_documents_path_reads_and_writes_nothing() {
    require_display();
    let scratch = std::env::temp_dir().join(format!("recomp_app_documents_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let documents = scratch.join("Documents");
    let local = scratch.join("AppData/Local");
    let games = documents.join("My Games/Oblivion");
    let plugins = local.join("Oblivion");
    std::fs::create_dir_all(&games).unwrap();
    std::fs::create_dir_all(&plugins).unwrap();
    let config = fixtures_root().join("config");
    std::fs::copy(config.join("Oblivion.ini"), games.join("Oblivion.ini")).unwrap();
    std::fs::copy(
        config.join("BlendSettings.ini"),
        games.join("BlendSettings.ini"),
    )
    .unwrap();
    std::fs::copy(config.join("Plugins.txt"), plugins.join("Plugins.txt")).unwrap();
    let prefix = scratch.join("prefix");
    let windows = prefix.join("drive_c/windows");
    std::fs::create_dir_all(&windows).unwrap();
    std::fs::copy(
        config.join("BlendSettings-version.ini"),
        windows.join("BlendSettings.ini"),
    )
    .unwrap();
    let list = |root: &Path| -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(folder) = pending.pop() {
            for entry in std::fs::read_dir(folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    found.push(path);
                }
            }
        }
        found.sort();
        found
    };
    let before = list(&scratch);
    let ini_before = std::fs::read(games.join("Oblivion.ini")).unwrap();
    let output = app()
        .env("OBLIVION_DOCUMENTS", &documents)
        .env("OBLIVION_LOCAL_APP_DATA", &local)
        .env("WINEPREFIX", &prefix)
        .args(["--seed", "1", "--exit-after-frames", "5"])
        .output()
        .unwrap();
    let after = list(&scratch);
    let ini_after = std::fs::read(games.join("Oblivion.ini")).unwrap();
    let _ = std::fs::remove_dir_all(&scratch);
    assert_g3(&output);
    assert_eq!(before, after, "no file created");
    assert_eq!(ini_before, ini_after, "the INI is untouched");
}

// A missing data folder runs every block, then has no MainMenu to open: exit 1 and a line that says so, not a black window.
#[test]
fn a_missing_data_folder_exits_1_and_says_why() {
    require_display();
    let scratch = std::env::temp_dir().join(format!("recomp_app_nodata_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let output = app()
        .env("OBLIVION_DATA_DIR", scratch.join("Data"))
        .arg("--ini")
        .arg(fixtures_root().join("config/Oblivion.ini"))
        .args(["--seed", "1", "--exit-after-frames", "3"])
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&scratch);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("No MainMenu:"), "{stderr}");
    assert!(stdout.contains("Phase-12: files compiled 0"), "{stdout}");
    assert!(stdout.contains("Phase-16: cell 0"), "{stdout}");
}

// The pinned INI's four intro stubs still run at 005ADE80 here, where the original's have ended: the draw comes when they do.
// The original shows the LoadingMenu through its load (linux-capture-2026-11-06, run-c1-log/frames).
#[test]
fn the_loading_menu_draws_with_the_intro_movies_running() {
    require_display();
    let output = app()
        .arg("--ini")
        .arg(fixtures_root().join("config/Oblivion.ini"))
        .args(["--no-audio-device", "--exit-after-frames", "200"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(
        stdout.contains("LoadingMenu: first frame drawn from a request"),
        "{stdout}"
    );
}
