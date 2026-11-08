//! Oblivion on Bevy. `--capture-frame N` before a capture flag holds the menu movie on its own frame N, as the C++ flag's did (`--menu-capture` alone takes 288), and captures draw no cursor. The capture flags (`--menu-capture PATH`, `--menu-capture-selected PATH`, `--loading-capture PATH STEPS`, `--message-capture continue|new|exit[-focused] PATH`, `--load-capture list|confirm PATH`, `--new-game-capture prison|overview|stats|inventory|magic|map PATH`: New Game, then the prison's 3D (from the player or above) or a built menu at the scripts' stop) take a PNG of a screen and exit. `--ini PATH` picks a pinned INI and its folder (the user's Documents folder by default), `--seed N` fixes the CRT's `_time64` seed and GetTickCount, `--exit-after-frames N` exits N updates after the WinMain driver ends, `--no-audio-device` never opens the machine's audio device, `--dump-movie-frames DIR` writes movie frames as `.ppm` files, and `OBLIVION_DATA_DIR` names the data folder (`Data` by default, as the original's process directory holds it).

use bevy::prelude::AppExit;
use recomp_host::HostOptions;
use recomp_host::screens::{CapturePlan, CaptureScript, NewGameView};
use recomp_host::winmain_plugin::DEFAULT_FRAME_BUDGET_MS;
use recomp_startup::winmain::{SettingsSource, WinMainConfig};
use std::path::PathBuf;
use std::time::Duration;

// IDA: N/A the host's entry point.
fn main() -> AppExit {
    let mut settings = SettingsSource::UserDocuments;
    let mut exit_after = None;
    let mut seed = None;
    let mut budget = Some(Duration::from_millis(DEFAULT_FRAME_BUDGET_MS));
    let mut capture = None;
    // The C++ flag's frame: `--capture-frame` before the capture's own flag. `--menu-capture` alone takes 288, the rest whatever is up.
    let mut movie_frame = None;
    let mut no_audio_device = false;
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
                });
            }
            "--intro-capture" => {
                let path = arguments.next().expect("a capture takes a path");
                budget = Some(Duration::ZERO);
                capture = Some(CapturePlan {
                    path: path.into(),
                    script: CaptureScript::Intro,
                    movie_frame: None,
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
                });
            }
            "--new-game-capture" => {
                let view = match arguments
                    .next()
                    .expect("prison, overview, stats, inventory, magic or map")
                    .as_str()
                {
                    "prison" => NewGameView::Prison,
                    "overview" => NewGameView::PrisonOverview,
                    "stats" => NewGameView::Menu(1003),
                    "inventory" => NewGameView::Menu(1002),
                    "magic" => NewGameView::Menu(1022),
                    "map" => NewGameView::Menu(1023),
                    other => panic!(
                        "--new-game-capture takes prison, overview, stats, inventory, magic or map, not {other}"
                    ),
                };
                let path = arguments.next().expect("a capture takes a path");
                capture = Some(CapturePlan {
                    path: path.into(),
                    script: CaptureScript::NewGame(view),
                    movie_frame: None,
                });
            }
            "--no-audio-device" => no_audio_device = true,
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
    let data_directory =
        std::env::var_os("OBLIVION_DATA_DIR").map_or_else(|| PathBuf::from("Data"), PathBuf::from);
    recomp_host::host_app(HostOptions {
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
    })
    .run()
}
