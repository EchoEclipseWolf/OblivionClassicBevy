//! Phase 07: WinMain 0040EB16..0040EB2C, the creation of the InterfaceManager and the LoadingMenu before game-data
//! construction. The classes are in `recomp_game::tes`. Runtime evidence: P07-RUN-001.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_game::gamebryo::scene_update::SceneUpdateRuntime;
use recomp_game::tes::interface_manager::{InterfaceSettings, run_interface_manager_get_singleton};
use recomp_game::tes::tile::InterfaceHost;

/// IDA: WinMain 0040EB16 push 1; 0040EB18 00578CC0 InterfaceManager_GetSingleton_w: push [esp+4] (= 1), push 1,
/// 00582160(1, 1), add esp, 8. The next instructions belong to Phase 08 (0040EB1D, 0040EB22 load its arguments;
/// 0040EB28 pops Phase 06's three 00410E40
/// arguments and this push; 0040EB2C calls 00406360). Completion: 100% of the order. Before this runs, the World must hold the `InterfaceMovieReads`
/// clone the MoviePlayer thread holds, the `SceneGraphGlobals` and the `GameSettingCollection`.
pub fn run_winmain_interface_window(
    world: &mut World,
    settings: &InterfaceSettings,
    host: &mut dyn InterfaceHost,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<(), StreamError> {
    run_interface_manager_get_singleton(world, settings, host, runtime, true, true)?;
    Ok(())
}
