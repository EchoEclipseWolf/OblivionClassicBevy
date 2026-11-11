//! The media seams over the real files: the real movie thread with FFmpeg's frames through the queue and the compositor, the menu-owned path, the splash, skipping with a key, and the C++ `audio_assets` over the real archives. No device and no window: only the virtual display, as every test. `RECOMP_DUMP_DIR` writes the frames looked at.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oblivion_host::audio::{decode_all, probe_decode, ui_asset_for_id};
use oblivion_host::ffmpeg_bink::FfmpegBink;
use oblivion_host::movie_frames::{MovieFrameCompositor, load_splash_with_ffmpeg, write_ppm};
use oblivion_sim::bs::archive::{ArchiveManager, ArchiveSettings};
use oblivion_sim::bs::file_finder::BsFileCallbacks;
use oblivion_sim::bs::file_finder::{FileFinder, bs_file_open_core};
use oblivion_sim::tes::bink_library::BinkLibrary;
use oblivion_sim::tes::interface_manager::InterfaceMovieReads;
use oblivion_sim::tes::movie_player::{
    HostMoviePlatform, MovieOsGlobals, MoviePlayer, MoviePlayerServices, MovieQueueEntry,
    MovieSettings, QueuedMovieDevice, finish_current_movie, movie_player_poll_wait_for_thread,
    movie_player_shutdown, play_menu_movie, play_movie, present_current_movie, show_movie_splash,
    start_async_movie_sequence,
};
use oblivion_sim::tes::os_globals::{
    GameSoundPause, HostInstance, OsInputGlobals, Shared, lock_shared, scripted_input_settings,
};
use oblivion_sim::tes::window::SharedTimeInfo;
use oblivion_sim::tes::winmain::winmain_host::HostOsPlatform;

fn data() -> PathBuf {
    std::env::var_os("OBLIVION_DATA_DIR")
        .map(PathBuf::from)
        .expect("OBLIVION_DATA_DIR names the game data")
}

struct Rig {
    player: MoviePlayer,
    bink: Arc<Mutex<FfmpegBink>>,
    receiver: Receiver<MovieQueueEntry>,
    os_platform: Arc<Mutex<HostOsPlatform>>,
    compositor: MovieFrameCompositor,
}

fn settings(sequence: &str, wait: f32) -> MovieSettings {
    MovieSettings {
        sound_enabled_b16178: true,
        preload_intro_sequence_b030b4: true,
        preload_size_limit_b030bc: 26_214_400,
        intro_sequence_priority_b030c4: 3,
        static_screen_wait_time_b030ac: wait,
        intro_sequence_b03084: sequence.as_bytes().to_vec(),
    }
}

fn rig(video_directory: &Path, sequence: &str, wait: f32) -> Rig {
    let bink = Arc::new(Mutex::new(FfmpegBink::default()));
    let (sender, receiver) = channel();
    let device =
        QueuedMovieDevice::new(sender).with_splash_loader(Some(Arc::new(load_splash_with_ffmpeg)));
    let os_platform = Arc::new(Mutex::new(HostOsPlatform::default()));
    let input = {
        let settings = scripted_input_settings(true);
        let mut platform = lock_shared(&os_platform);
        OsInputGlobals::new(HostInstance(0x0040_0000), &settings, &mut *platform).unwrap()
    };
    let player = MoviePlayer::new(MoviePlayerServices {
        bink: bink.clone(),
        device: Some(Arc::new(Mutex::new(device))),
        platform: Arc::new(Mutex::new(HostMoviePlatform::default())),
        os_platform: os_platform.clone(),
        renderer_width: 1920,
        renderer_height: 1080,
        os_globals: Some(MovieOsGlobals {
            input_20: Some(Shared::new(*input)),
            sound_24: Some(GameSoundPause::default()),
            main_thread_id_10: oblivion_sim::tes::movie_player::host_thread_id(),
            main_thread_handle_14: 0,
        }),
        time: SharedTimeInfo::default(),
        interface: InterfaceMovieReads::default(),
        video_directory: video_directory.to_path_buf(),
        settings: settings(sequence, wait),
    });
    Rig {
        player,
        bink,
        receiver,
        os_platform,
        compositor: MovieFrameCompositor::default(),
    }
}

impl Rig {
    // The queue into the compositor, as each frame's drain does; `on_frame` sees each new picture.
    fn drain(&mut self, mut on_frame: impl FnMut(&MovieFrameCompositor)) {
        let mut seen = self.compositor.frame.as_ref().map(|frame| frame.generation);
        while let Ok(entry) = self.receiver.try_recv() {
            self.compositor.apply(entry);
            let now = self.compositor.frame.as_ref().map(|frame| frame.generation);
            if now.is_some() && now != seen {
                seen = now;
                on_frame(&self.compositor);
            }
        }
    }
}

fn dump(name: &str, width: u32, height: u32, bgra: &[u8]) {
    if let Some(folder) = std::env::var_os("RECOMP_DUMP_DIR") {
        let folder = PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        write_ppm(&folder.join(format!("{name}.ppm")), width, height, bgra).unwrap();
    }
}

/// The real thread (`std::thread`, through 00410E40) over the shipped `sIntroSequence`: four stubs, each one real decoded frame of 720x480 through the tile set, as P06-RUN-001 opened them. The C++ `verify_local_stub`, and its `decoded playback frames` item.
#[test]
fn the_intro_sequence_plays_on_the_real_thread() {
    let sequence = "bethesda softworks HD720p.bik,2k games.bik,game studios.bik,Oblivion Legal.bik";
    let mut rig = rig(&data().join("Video"), sequence, 0.0);
    let result = start_async_movie_sequence(&rig.player, sequence.as_bytes(), false, -1);
    assert_eq!(result, 1, "a suspended thread resumes once");
    let owner = lock_shared(&rig.player.globals.host_thread_owner)
        .clone()
        .unwrap();
    assert_ne!(
        owner.thread_id(),
        Some(std::thread::current().id()),
        "a real thread"
    );
    let mut frames = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    while owner.is_active() {
        assert!(Instant::now() < deadline, "the thread didn't end");
        rig.drain(|compositor| frames.push(compositor.frame.clone().unwrap()));
        std::thread::sleep(Duration::from_millis(2));
    }
    owner.wait();
    rig.drain(|compositor| frames.push(compositor.frame.clone().unwrap()));
    assert_eq!(frames.len(), 4, "one frame from each stub");
    assert_eq!(rig.compositor.presents, 4);
    for (index, frame) in frames.iter().enumerate() {
        // 0040FEE0's grid for 720x480: 2 full columns and a partial one of 256; a full row and a partial one of 256.
        assert_eq!((frame.width, frame.height), (768, 512));
        assert_eq!(frame.screen_position, [150.0, 0.0], "P06-RUN-001's x");
        assert_eq!(frame.screen_size, [768.0 * 2.25, 512.0 * 2.25]);
        dump(
            &format!("intro-{index}"),
            frame.width,
            frame.height,
            &frame.bgra,
        );
    }
    assert!(lock_shared(&rig.player.globals.thread_b33434).is_none());
    assert!(rig.compositor.frame.is_none(), "every movie closed");
    assert!(!lock_shared(&rig.bink).sound_system_set);
    movie_player_shutdown(&rig.player);
}

/// A menu-owned `Map loop.bik` at 1.5 on the main thread, frames at their real time, each one really different. Its row is `OBJECT|MENU` in P06-RUN-001: scale 3FC00000, x and y 0.
#[test]
fn map_loop_plays_as_the_menu_owns_it() {
    let mut rig = rig(&data().join("Video"), "", 0.0);
    assert!(play_menu_movie(&rig.player, "Map loop.bik", true));
    let movie = lock_shared(&rig.player.globals.current_b33428)
        .clone()
        .unwrap();
    assert_eq!(movie.state(), 2);
    {
        let fields = movie.fields();
        assert_eq!(fields.scale_14.to_bits(), 0x3FC0_0000);
        assert_eq!(fields.x_18, 0.0);
        assert_eq!(fields.y_1c, 0.0);
    }
    let mut pictures = Vec::new();
    let started = Instant::now();
    while pictures.len() < 45 {
        assert!(started.elapsed() < Duration::from_secs(60));
        assert!(present_current_movie(&rig.player, false, true), "it loops");
        rig.drain(|compositor| pictures.push(compositor.frame.clone().unwrap()));
        std::thread::sleep(Duration::from_millis(1));
    }
    let elapsed = started.elapsed().as_secs_f64();
    let first = &pictures[0];
    assert_eq!((first.width, first.height), (1280, 768));
    assert_eq!(first.screen_position, [0.0, 0.0]);
    assert_eq!(first.screen_size, [1920.0, 1152.0]);
    assert_eq!(
        rig.compositor.presents, 0,
        "the menu presents, not the movie"
    );
    assert_ne!(first.bgra, pictures[44].bgra, "the picture moves");
    // 44 frame times at 2997/100 is 1.468 s. It can't be early; a loaded machine can be late.
    assert!(elapsed >= 1.4, "{elapsed}");
    dump("maploop-000", first.width, first.height, &first.bgra);
    dump("maploop-044", 1280, 768, &pictures[44].bgra);
    // Below row 720 is 0040F724's zeroed memory: black and opaque.
    let below = (730 * 1280 + 10) * 4;
    assert_eq!(&first.bgra[below..below + 4], &[0, 0, 0, 255]);
    finish_current_movie(&rig.player);
    rig.drain(|_| {});
    assert!(rig.compositor.frame.is_none());
}

/// The loop itself: past frame 750 the movie is back at 1 with its first picture.
#[test]
fn map_loop_wraps_after_its_last_frame() {
    let path = data().join("Video/Map loop.bik");
    let mut bink = FfmpegBink::default();
    let handle = bink.open(&path, 0).unwrap();
    let mut first = vec![0; 1280 * 720 * 4];
    let mut last = vec![0; 1280 * 720 * 4];
    let mut wrapped = vec![0; 1280 * 720 * 4];
    let copy = |bink: &mut FfmpegBink, into: &mut [u8]| {
        bink.do_frame(handle);
        bink.copy_to_buffer_rect(handle, into, 1280 * 4, 720, 0, 0, 0, 0, 1280, 720, 3);
    };
    copy(&mut bink, &mut first);
    for _ in 1..750 {
        bink.next_frame(handle);
    }
    assert_eq!(bink.info(handle).frame_num_0c, 750);
    copy(&mut bink, &mut last);
    assert_ne!(first, last);
    bink.next_frame(handle);
    assert_eq!(bink.info(handle).frame_num_0c, 1);
    copy(&mut bink, &mut wrapped);
    assert_eq!(first, wrapped);
    dump("maploop-750", 1280, 720, &last);
    bink.close(handle);
}

/// A key on the thread's own poll: Escape in `HostInputFrame`, as `raw_input` feeds it, stops the long movie and skips the rest (00410390's controls 29 and 5, 00B33426).
#[test]
fn escape_skips_the_sequence() {
    let sequence = "Map loop.bik,2k games.bik";
    let mut rig = rig(&data().join("Video"), sequence, 0.0);
    start_async_movie_sequence(&rig.player, sequence.as_bytes(), false, -1);
    let owner = lock_shared(&rig.player.globals.host_thread_owner)
        .clone()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut frames = 0;
    while frames < 3 {
        assert!(Instant::now() < deadline);
        rig.drain(|_| frames += 1);
        std::thread::sleep(Duration::from_millis(2));
    }
    {
        let platform = lock_shared(&rig.os_platform);
        lock_shared(&platform.input).keyboard[0x01] = 0x80;
    }
    while owner.is_active() {
        assert!(Instant::now() < deadline, "Escape didn't end the thread");
        rig.drain(|_| frames += 1);
        std::thread::sleep(Duration::from_millis(2));
    }
    owner.wait();
    rig.drain(|_| frames += 1);
    assert!(frames < 200, "stopped well before the end: {frames}");
    assert_eq!(
        rig.player
            .globals
            .skip_requested_b33426
            .load(Ordering::Acquire),
        0,
        "00410E14 clears it at the sequence's end"
    );
    movie_player_shutdown(&rig.player);
}

fn wait_for_movies_done(rig: &mut Rig) {
    rig.drain(|_| {});
    assert!(movie_player_poll_wait_for_thread(&rig.player));
}

/// The C++ `verify_splash`: a missing file returns at once, and a real 8x4 BGRA DDS is decoded, drawn at its height fit, presented, and released after the wait. Its pixels are the file's.
#[test]
fn a_splash_decodes_draws_and_waits() {
    let folder = std::env::temp_dir().join(format!("recomp_splash_{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let mut rig = rig(&folder, "", 0.05);
    assert_ne!(show_movie_splash(&rig.player, "missing.dds"), 0);
    let mut header = [0u32; 32];
    header[0] = 0x2053_4444;
    header[1] = 124;
    header[2] = 0x100F;
    header[3] = 4;
    header[4] = 8;
    header[7] = 1;
    header[19] = 32;
    header[20] = 0x41;
    header[22] = 32;
    header[23] = 0x00FF_0000;
    header[24] = 0x0000_FF00;
    header[25] = 0x0000_00FF;
    header[26] = 0xFF00_0000;
    let mut file: Vec<u8> = header.iter().flat_map(|word| word.to_le_bytes()).collect();
    for _ in 0..8 * 4 {
        file.extend_from_slice(&0xFF33_6699u32.to_le_bytes());
    }
    std::fs::write(folder.join("splash.dds"), file).unwrap();
    let started = Instant::now();
    assert_eq!(show_movie_splash(&rig.player, "splash.dds"), 0);
    assert!(
        started.elapsed() >= Duration::from_millis(40),
        "the wait ran"
    );
    let mut pictures = Vec::new();
    rig.drain(|compositor| pictures.push(compositor.frame.clone().unwrap()));
    assert_eq!(pictures.len(), 1);
    let picture = &pictures[0];
    assert_eq!((picture.width, picture.height), (8, 4));
    assert_eq!(&picture.bgra[..4], &[0x99, 0x66, 0x33, 0xFF]);
    // 1080 / 4 is 270; 8 * 270 is wider than 1920, so x is negative, and the quad is 1920 - 2x wide.
    assert_eq!(picture.screen_position, [-120.0, 0.0]);
    assert_eq!(picture.screen_size, [2160.0, 1080.0]);
    assert_eq!(rig.compositor.presents, 1);
    assert!(rig.compositor.frame.is_none(), "00410A53 released it");
    wait_for_movies_done(&mut rig);
    std::fs::remove_dir_all(&folder).unwrap();
}

/// A file that isn't there is a failed open, through to the quiet play's false, with nothing left behind.
#[test]
fn a_missing_movie_plays_nothing() {
    let folder = std::env::temp_dir().join(format!("recomp_nomovie_{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let mut rig = rig(&folder, "", 0.0);
    assert!(!play_movie(
        &rig.player,
        "none.bik",
        true,
        true,
        false,
        false,
        false,
        false
    ));
    assert!(lock_shared(&rig.player.globals.current_b33428).is_none());
    assert!(!play_menu_movie(&rig.player, "none.bik", true));
    rig.drain(|_| {});
    assert!(rig.compositor.frame.is_none());
    std::fs::remove_dir_all(&folder).unwrap();
}

// ---- the C++ `audio_assets` ----

struct Assets {
    finder: FileFinder,
    manager: ArchiveManager,
}

impl Assets {
    fn new() -> Self {
        let data = data();
        let root = data.parent().unwrap().to_path_buf();
        let mut callbacks = BsFileCallbacks::default();
        let mut finder = FileFinder::new(&root, &mut callbacks);
        finder.add(data.file_name().unwrap().to_str().unwrap());
        let settings = ArchiveSettings {
            archive_list: b"Oblivion - Sounds.bsa, Oblivion - Voices1.bsa".to_vec(),
            ..ArchiveSettings::default()
        };
        let mut manager = ArchiveManager::new(&root, settings);
        manager.initialize_from_settings(&finder, &root.join("no-local-folder"));
        Self { finder, manager }
    }

    fn read(&mut self, path: &str) -> Option<Arc<[u8]>> {
        let mut file = bs_file_open_core(
            &self.finder,
            Some(&mut self.manager),
            path,
            0,
            0x2800,
            0xFFFF,
        )?;
        let mut bytes = vec![0; file.size() as usize];
        (file.read(&mut bytes) == bytes.len()).then(|| bytes.into())
    }
}

fn check(bytes: Arc<[u8]>, channels: u16) {
    let probe = probe_decode(bytes).unwrap();
    assert_eq!(probe.channels, channels);
    assert_eq!(probe.sample_rate, 44100);
    assert!(probe.decoded_frames > 1000, "{}", probe.decoded_frames);
    assert!(probe.non_silent);
}

#[test]
fn real_assets_decode_with_their_formats() {
    let mut assets = Assets::new();
    let title = std::fs::read(data().join("Music/Special/tes4title.mp3")).unwrap();
    check(title.into(), 2);
    let focus = assets
        .read(&ui_asset_for_id(4, 0).unwrap())
        .expect("the focus sound is in Oblivion - Sounds.bsa");
    check(focus, 1);
    for id in [1, 2] {
        let sound = assets.read(&ui_asset_for_id(id, 0).unwrap()).unwrap();
        assert!(decode_all(sound).unwrap().samples.len() > 100, "{id}");
    }
    let voice = assets
        .read("Sound\\voice\\oblivion.esm\\high elf\\f\\ms14_ms14returnhome_0001c1cd_1.mp3")
        .expect("the voice is in Oblivion - Voices1.bsa");
    check(voice, 1);
}

/// The real music, whole, against an independent decoder: the same format, and the same samples to within what two MP3 decoders differ by, after aligning for the decoder delay each trims differently.
#[test]
fn title_music_matches_ffmpeg() {
    let path = data().join("Music/Special/tes4title.mp3");
    let ours = decode_all(std::fs::read(&path).unwrap().into()).unwrap();
    assert_eq!((ours.channels, ours.sample_rate), (2, 44100));
    let output = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(&path)
        .args(["-f", "f32le", "-ac", "2", "-ar", "44100", "pipe:1"])
        .output()
        .unwrap();
    let theirs: Vec<f32> = output
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|sample| f32::from_le_bytes(*sample))
        .collect();
    let difference = ours.samples.len().abs_diff(theirs.len()) / 2;
    println!(
        "frames: ours {}, ffmpeg {}",
        ours.samples.len() / 2,
        theirs.len() / 2
    );
    assert!(difference <= 2 * 1152, "{difference} frames apart");
    // The middle of the track, where neither decoder's start matters: the best offset within two MP3 frames.
    let at = (ours.samples.len() / 2) & !1;
    let window = 44100;
    let error = |offset: isize| {
        let start = (at as isize + offset * 2) as usize;
        ours.samples[at..at + window]
            .iter()
            .zip(&theirs[start..start + window])
            .map(|(a, b)| f64::from((a - b).abs()))
            .sum::<f64>()
            / window as f64
    };
    let best = (-2400..=2400).map(error).fold(f64::MAX, f64::min);
    println!("mean absolute difference at the best offset: {best}");
    assert!(best < 0.001, "{best}");
}

#[test]
fn a_missing_archived_file_is_a_failed_lookup() {
    let mut assets = Assets::new();
    assert!(assets.read("Sound\\missing.wav").is_none());
}
