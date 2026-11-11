//! Ported original game code, by engine layer. `oblivion_sim` uses `bevy_ecs` but no Bevy
//! rendering, audio or windowing. Mappings are in `docs/ecs-mapping.md`.

pub mod bs;
pub mod gamebryo;
pub mod tes;

/// IDA: N/A the simulation's own App (restructure Stage 8a): `App::new()` with no Bevy plugins, the WinMain driver
/// installed on its world (`install_winmain`), the `GameFrame` schedule's ordered sets added, and `SimPlugins` filling
/// them (Stage 10). The host holds it and steps it once per frame;
/// tests may keep building a bare `World` with `install_winmain`. Completion: 100%.
pub fn sim_app(
    config: tes::winmain::WinMainConfig,
    seams: tes::winmain::WinMainSeams,
) -> bevy_app::App {
    let mut app = bevy_app::App::new();
    tes::winmain::install_winmain(app.world_mut(), config, seams);
    app.add_schedule(tes::winmain::game_frame::empty_game_frame());
    app.add_plugins(tes::winmain::game_frame::SimPlugins);
    app
}
