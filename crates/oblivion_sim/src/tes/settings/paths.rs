//! The paths WinMain 0040DFB2..0040E186 builds (see `crate::tes::winmain::path_setup`).
//!
//! Recorded host substitution: the original uses `-A` APIs on char[0x104] buffers. Paths here are
//! host paths, never original pointers or fixed-width layouts.

use std::path::{Path, PathBuf};

use bevy_ecs::prelude::*;
use oblivion_core::profile;

/// IDA: 00B3F178 local data, 00B3F280 settings, and the selected INI. Later phases read them.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct GamePaths {
    pub use_my_games: bool,
    pub bootstrap_ini: PathBuf,
    pub settings_directory: PathBuf,
    pub local_data_directory: PathBuf,
    pub selected_ini: PathBuf,
    pub saves_directory: PathBuf,
}

/// IDA: an original relative path, with its backslashes, as a host path.
#[must_use]
pub fn host_relative_path(original: &[u8]) -> PathBuf {
    let text: String = original.iter().map(|&byte| char::from(byte)).collect();
    text.replace('\\', "/").into()
}

/// IDA: 009DB750 SLocalSavePath default "Saves\". The original consumer 0045D450 builds a literal
/// concatenation with no validation. Host-owned save index: a relative directory under the settings
/// root; rooted or escaping values are an error. `OBLIVION_SAVES_DIR` overrides it (IDA: N/A).
/// Completion: 100% for the host save-index policy.
/// TODO: [ ] MainMenu phase: port 0045D450's concatenation and its silent FindFirstFileA failure.
pub fn host_saves_directory(paths: &GamePaths) -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("OBLIVION_SAVES_DIR") {
        return Ok(PathBuf::from(value));
    }
    saves_directory_from_ini(&paths.selected_ini, &paths.settings_directory)
}

// IDA: N/A a helper, with no address of its own.
fn saves_directory_from_ini(
    selected_ini: &Path,
    settings_directory: &Path,
) -> Result<PathBuf, String> {
    let value = profile::read_string(
        selected_ini,
        b"General",
        b"SLocalSavePath",
        Some(b"Saves\\"),
        1024,
    );
    if value.len() >= 1023 {
        return Err("Oblivion.ini save path is too long".to_owned());
    }
    let relative = if value.is_empty() {
        PathBuf::from("Saves")
    } else {
        host_relative_path(&value)
    };
    if relative.has_root() {
        return Err("Oblivion.ini save path must be relative".to_owned());
    }
    if relative.components().any(|part| part.as_os_str() == "..") {
        return Err("Oblivion.ini save path escapes settings directory".to_owned());
    }
    Ok(settings_directory.join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_directory_rules() {
        let directory = std::env::temp_dir().join(format!("recomp_saves_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let ini = directory.join("Oblivion.ini");
        let root = Path::new("/games");

        assert_eq!(
            saves_directory_from_ini(&ini, root).unwrap(),
            root.join("Saves/")
        );
        std::fs::write(&ini, "[General]\nSLocalSavePath=Other\\Saves\\\n").unwrap();
        assert_eq!(
            saves_directory_from_ini(&ini, root).unwrap(),
            root.join("Other/Saves/")
        );
        std::fs::write(&ini, "[General]\nSLocalSavePath=..\\Saves\\\n").unwrap();
        assert!(saves_directory_from_ini(&ini, root).is_err());
        std::fs::write(&ini, "[General]\nSLocalSavePath=\\Saves\\\n").unwrap();
        assert!(saves_directory_from_ini(&ini, root).is_err());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
