//! Phase 03: WinMain 0040E6BE..0040E757, the registry and INI setting loads.

use std::path::Path;

use crate::tes::settings::SettingCollections;
use oblivion_core::logging::{Level, emit};

/// IDA: A30FEC g_oblivionRegistryKeyLiteral.
const REGISTRY_KEY: &str = "Software\\Bethesda Softworks\\Oblivion";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WinMainSettingLoad {
    pub registry_opened: bool,
    pub registry_read_all: bool,
    pub ini_opened: bool,
    pub ini_read_all: bool,
}

/// IDA: 0040E6BE..0040E757: copy the key, registry vt+0x14(0) and, on success, vt+0x20 then vt+0x18;
/// copy the selected INI path, INI vt+0x14(0) and, on success, vt+0x20 then vt+0x18. Both ReadAll
/// results are ignored. Recorded host substitution: the registry open fails on Linux, so its arm
/// stops at the open. Completion: 100% order.
pub fn load_winmain_settings(
    collections: &mut SettingCollections,
    selected_ini: &Path,
) -> WinMainSettingLoad {
    let mut result = WinMainSettingLoad::default();
    collections.registry.set_path(REGISTRY_KEY);
    // 004A8C90: no registry here.
    result.registry_opened = false;
    collections.ini.set_path(selected_ini);
    result.ini_opened = collections.ini.open_ini();
    if result.ini_opened {
        result.ini_read_all = collections.ini.read_all();
        collections.ini.close();
    }
    emit(
        Level::Verbose,
        &format!(
            "Settings: registry not opened, INI {} settings read",
            collections.ini.settings().len()
        ),
    );
    result
}
