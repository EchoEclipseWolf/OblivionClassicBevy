//! The startup sequence: WinMain blocks, `Startup_*` routines and startup-only helpers. Classes a
//! startup block constructs belong to their layer in `recomp_game`.

pub mod initialize_game_data;
pub mod initialize_scene_graphs;
pub mod path_setup;
pub mod plugin_selection;
pub mod scaffold;
#[allow(clippy::all)]
mod scaffold_data;
pub mod winmain;
pub mod winmain_host;
pub mod winmain_initial_area;
pub mod winmain_interface;
pub mod winmain_memory;
pub mod winmain_menu_media;
pub mod winmain_movie;
pub mod winmain_place_player;
pub mod winmain_player_scene;
pub mod winmain_renderer;
pub mod winmain_scene;
pub mod winmain_screens;
pub mod winmain_settings;
pub mod winmain_shader;
pub mod winmain_starting_spell;
pub mod winmain_window;
pub mod winmain_world_setup;
