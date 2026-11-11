//! Oblivion on Bevy. `--capture-frame N` before a capture flag holds the menu movie on its own frame N, as the C++ flag's did (`--menu-capture` alone takes 288), and captures draw no cursor. The capture flags (`--menu-capture PATH`, `--menu-capture-selected PATH`, `--loading-capture PATH STEPS`, `--message-capture continue|new|exit[-focused] PATH`, `--load-capture list|confirm PATH`, `--new-game-capture prison|overview|actor ID|stats|inventory|magic|map|race PATH`: New Game, then the prison's 3D (from the player or above) or a built menu at the scripts' stop) take a PNG of a screen and exit. `--ini PATH` picks a pinned INI and its folder (the user's Documents folder by default), `--seed N` fixes the CRT's `_time64` seed and GetTickCount, `--exit-after-frames N` exits N updates after the WinMain driver ends, `--race-click PANE CHILD ARROW TIMES` (repeatable) clicks a race menu tile's arrow (`-` the child itself; pane `Message` a shown box's button by ID; pane `Name` the name tile with the child typed) that many times before a `race` capture's view is built (ten interface frames after each), `--race-set PANE CHILD VALUE` (repeatable, in order with the clicks) stores a race menu slider's value as the game's slider store does, `--live-click`, `--live-drag DX`, `--live-resize W H`, `--live-shot NAME` and `--live-exit` (repeatable, in order) run on a `--new-game-view` once it's up (docs/live-tuning.md), `--new-game-view MODE` runs a `--new-game-capture` mode's steps and then keeps the window open for live tuning (F1 the tuning panel, PageDown/PageUp the views, F9 a fly camera, F12 a screenshot), `--tuning-panel` opens the tuning panel at startup, `--tuning PATH` names the tuning file (`config/tuning.ron` by default, loaded at startup by every run), `--no-audio-device` never opens the machine's audio device, `--press-new-game` presses the main menu's New Game in a plain run once the menu has settled (a check with no mouse; `--press-new-game-shot PATH` also writes the window 60 updates after the world view is up and exits), `--dump-movie-frames DIR` writes movie frames as `.ppm` files, and `OBLIVION_DATA_DIR` names the data folder (`Data` by default, as the original's process directory holds it).
//! `--capture-tick-ms N` advances GetTickCount by N milliseconds per host frame. Pair it with `--seed` and `--no-frame-budget` for repeatable New Game captures.

use bevy::prelude::{AppExit, First};
use oblivion_host::HostOptions;
use oblivion_host::screens::{CapturePlan, CaptureScript, NewGameView};
use oblivion_host::winmain_plugin::DEFAULT_FRAME_BUDGET_MS;
use oblivion_sim::tes::winmain::{SettingsSource, WinMainConfig};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

// IDA: N/A the host's entry point.
fn main() -> AppExit {
    let mut settings = SettingsSource::UserDocuments;
    let mut exit_after = None;
    let mut seed = None;
    let mut capture_tick_ms = None;
    let mut budget = Some(Duration::from_millis(DEFAULT_FRAME_BUDGET_MS));
    let mut tuning: Option<PathBuf> = None;
    let mut tuning_panel = false;
    let mut capture = None;
    let mut clicks = Vec::new();
    let mut live_script = Vec::new();
    // The C++ flag's frame: `--capture-frame` before the capture's own flag. `--menu-capture` alone takes 288, the rest whatever is up.
    let mut movie_frame = None;
    let mut no_audio_device = false;
    let mut press_new_game = None;
    let mut dump_movie_frames = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--ini" => {
                settings =
                    SettingsSource::Pinned(arguments.next().expect("--ini takes a path").into());
            }
            "--exit-after-frames" => {
                let frames = arguments.next().expect("--exit-after-frames takes a count");
                exit_after = Some(frames.parse().expect("a frame count"));
            }
            "--seed" => {
                let value = arguments.next().expect("--seed takes a number");
                seed = Some(value.parse().expect("a 32-bit seed"));
            }
            "--capture-tick-ms" => {
                let value = arguments
                    .next()
                    .expect("--capture-tick-ms takes milliseconds");
                capture_tick_ms = Some(value.parse::<NonZeroU32>().expect("a positive tick step"));
            }
            "--no-frame-budget" => budget = None,
            "--capture-frame" => {
                let frame = arguments.next().expect("--capture-frame takes a count");
                movie_frame = Some(frame.parse().expect("a frame count"));
            }
            "--menu-capture" | "--menu-capture-selected" => {
                let path = arguments.next().expect("a capture takes a path");
                capture = Some(CapturePlan {
                    path: path.into(),
                    script: if argument == "--menu-capture" {
                        CaptureScript::Menu
                    } else {
                        CaptureScript::MenuSelected
                    },
                    movie_frame: if argument == "--menu-capture" {
                        movie_frame.or(Some(288))
                    } else {
                        movie_frame
                    },
                    live: false,
                    clicks: Vec::new(),
                    live_script: Vec::new(),
                });
            }
            "--intro-capture" => {
                let path = arguments.next().expect("a capture takes a path");
                budget = Some(Duration::ZERO);
                capture = Some(CapturePlan {
                    path: path.into(),
                    script: CaptureScript::Intro,
                    movie_frame: None,
                    live: false,
                    clicks: Vec::new(),
                    live_script: Vec::new(),
                });
            }
            "--loading-capture" => {
                let path = arguments.next().expect("a capture takes a path");
                let percent = arguments.next().expect("a step count");
                budget = Some(Duration::ZERO);
                capture = Some(CapturePlan {
                    path: path.into(),
                    script: CaptureScript::Loading(percent.parse().expect("a step count")),
                    movie_frame: None,
                    live: false,
                    clicks: Vec::new(),
                    live_script: Vec::new(),
                });
            }
            "--message-capture" => {
                let kind = arguments
                    .next()
                    .expect("continue, new or exit, with -focused");
                let path = arguments.next().expect("a capture takes a path");
                let focused = kind.ends_with("-focused");
                let button = match kind.trim_end_matches("-focused") {
                    "continue" => 2,
                    "new" => 3,
                    "exit" => 7,
                    other => panic!("unknown message capture {other}"),
                };
                capture = Some(CapturePlan {
                    path: path.into(),
                    script: CaptureScript::Message { button, focused },
                    movie_frame,
                    live: false,
                    clicks: Vec::new(),
                    live_script: Vec::new(),
                });
            }
            "--load-capture" => {
                let kind = arguments.next().expect("list or confirm");
                let path = arguments.next().expect("a capture takes a path");
                capture = Some(CapturePlan {
                    path: path.into(),
                    script: CaptureScript::Load {
                        confirm: kind == "confirm",
                    },
                    movie_frame,
                    live: false,
                    clicks: Vec::new(),
                    live_script: Vec::new(),
                });
            }
            "--tuning-panel" => tuning_panel = true,
            "--tuning" => {
                tuning = Some(PathBuf::from(
                    arguments.next().expect("--tuning takes a RON file"),
                ));
            }
            "--new-game-capture" | "--new-game-view" => {
                let live = argument == "--new-game-view";
                let view = match arguments
                    .next()
                    .expect("prison, overview, actor, stats, inventory, magic, map or race")
                    .as_str()
                {
                    "prison" => NewGameView::Prison,
                    "actor" => NewGameView::Actor(
                        u32::from_str_radix(
                            &arguments
                                .next()
                                .expect("--new-game-capture actor takes a hex form ID"),
                            16,
                        )
                        .expect("a hex form ID"),
                    ),
                    "overview" => NewGameView::PrisonOverview,
                    "stats" => NewGameView::Menu(1003),
                    "inventory" => NewGameView::Menu(1002),
                    "magic" => NewGameView::Menu(1022),
                    "map" => NewGameView::Menu(1023),
                    "race" => NewGameView::Menu(1036),
                    other => panic!(
                        "--new-game-capture takes prison, overview, stats, inventory, magic, map or race, not {other}"
                    ),
                };
                let path = if live {
                    "screenshots/live.png".to_owned()
                } else {
                    arguments.next().expect("a capture takes a path")
                };
                capture = Some(CapturePlan {
                    path: path.into(),
                    script: CaptureScript::NewGame(view),
                    movie_frame: None,
                    live,
                    clicks: Vec::new(),
                    live_script: Vec::new(),
                });
            }
            "--race-click" => {
                let mut word = || {
                    arguments
                        .next()
                        .expect("--race-click takes PANE CHILD ARROW TIMES")
                };
                clicks.push(oblivion_host::screens::CaptureClick {
                    pane: word(),
                    child: word(),
                    arrow: word(),
                    times: word().parse().expect("a click count"),
                    value: None,
                });
            }
            "--live-click" => {
                let mut word = || {
                    arguments
                        .next()
                        .expect("--live-click takes PANE CHILD ARROW TIMES")
                };
                live_script.push(oblivion_host::screens::LiveStep::Click(
                    oblivion_host::screens::CaptureClick {
                        pane: word(),
                        child: word(),
                        arrow: word(),
                        times: word().parse().expect("a click count"),
                        value: None,
                    },
                ));
            }
            "--live-drag" => live_script.push(oblivion_host::screens::LiveStep::Drag(
                arguments
                    .next()
                    .expect("--live-drag takes a distance")
                    .parse()
                    .expect("a distance"),
            )),
            "--live-shot" => live_script.push(oblivion_host::screens::LiveStep::Shot(
                arguments.next().expect("--live-shot takes a name"),
            )),
            "--live-resize" => {
                let mut size = || {
                    arguments
                        .next()
                        .expect("--live-resize takes W H")
                        .parse()
                        .expect("a size in pixels")
                };
                let (width, height) = (size(), size());
                live_script.push(oblivion_host::screens::LiveStep::Resize(width, height));
            }
            "--live-exit" => live_script.push(oblivion_host::screens::LiveStep::Exit),
            "--race-set" => {
                let mut word = || arguments.next().expect("--race-set takes PANE CHILD VALUE");
                clicks.push(oblivion_host::screens::CaptureClick {
                    pane: word(),
                    child: word(),
                    arrow: String::new(),
                    times: 0,
                    value: Some(word().parse().expect("a slider value")),
                });
            }
            "--no-audio-device" => no_audio_device = true,
            "--press-new-game" => press_new_game = Some(None),
            "--press-new-game-shot" => {
                press_new_game = Some(Some(PathBuf::from(
                    arguments
                        .next()
                        .expect("--press-new-game-shot takes a path"),
                )));
            }
            "--dump-movie-frames" => {
                dump_movie_frames = Some(PathBuf::from(
                    arguments
                        .next()
                        .expect("--dump-movie-frames takes a folder"),
                ));
            }
            other => panic!("unknown argument {other}"),
        }
    }
    if let Some(plan) = capture.as_mut() {
        plan.clicks = clicks;
        plan.live_script = live_script;
    }
    let data_directory =
        std::env::var_os("OBLIVION_DATA_DIR").map_or_else(|| PathBuf::from("Data"), PathBuf::from);
    let mut app = oblivion_host::host_app(HostOptions {
        winmain: WinMainConfig {
            data_directory,
            settings,
            seed,
            frame_budget: budget,
        },
        exit_after_frames: exit_after,
        capture,
        no_audio_device,
        dump_movie_frames,
        tuning,
        tuning_panel,
    });
    if let Some(shot) = press_new_game {
        app.insert_resource(oblivion_host::screens::PressNewGame {
            shot,
            ..Default::default()
        });
    }
    if let Some(step) = capture_tick_ms {
        let clock = app
            .world()
            .non_send::<oblivion_host::winmain_plugin::SimWorld>()
            .0
            .world()
            .resource::<oblivion_sim::tes::winmain::winmain_host::HostClock>()
            .clone();
        let mut tick = clock.tick();
        app.add_systems(First, move || {
            clock.set_scripted_tick(Some(tick));
            tick = tick.wrapping_add(step.get());
        });
    }
    app.run()
}
