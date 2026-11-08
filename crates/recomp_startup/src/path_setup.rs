//! Phase 01: WinMain 0040DFB2..0040E3B7 path setup, the 00404940 disc check, and the Plugins.txt
//! path 00404B00 builds from 00B3F178 (Phase 10).
//!
//! Recorded host substitutions: `-A` APIs on char[0x104] become host paths, with overflow reported
//! rather than reproduced. The Known Folders are configured here, since Linux has none. The
//! 0040E39B second-instance check belongs to the host window (M2).

use std::path::{Path, PathBuf};

use recomp_core::host_path;
use recomp_core::logging::{Level, emit};
use recomp_core::profile;
use recomp_game::tes::paths::GamePaths;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathRoots {
    pub working: PathBuf,
    /// Empty when the folder lookup failed: 0040DFF9 / 0040E071 ignore the HRESULT and append to
    /// the empty string it leaves.
    pub documents: PathBuf,
    pub local_app_data: PathBuf,
}

/// IDA: 0040E043, 0040E0B8, 0040E0F3, 0040E1C1 and 0040E221 results. The original ignores them all;
/// the host reports them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StartupPathPreparation {
    /// CreateDirectoryA LocalAppData\Oblivion\, Documents\My Games\ and Documents\My Games\Oblivion\
    /// in original order; true = creation failed (an existing directory is not a failure).
    pub directory_failures: [bool; 3],
    pub selected_ini_existed: bool,
    pub default_copy_attempted: bool,
    pub default_copied: bool,
}

/// IDA: 0040DFF9..0040E186 the char[0x104] buffers. true = the original string would not fit with
/// its NUL. Measured in the code page's bytes, as `original_length` counts them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OriginalPathBufferOverflow {
    pub local_app_data: bool,
    pub my_games: bool,
    pub selected_ini: bool,
}

// IDA: N/A a helper, with no address of its own.
fn folder_root(folder: &Path) -> PathBuf {
    // An empty folder string makes the original path root-relative.
    if folder.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        folder.to_path_buf()
    }
}

/// IDA: N/A the C++ `ansi_length`, WideCharToMultiByte(CP_ACP): one byte per character of a UTF-8 path and the raw bytes
/// of any other. Host substitution: this assumes a single-byte code page, where a double-byte one would count more.
fn original_length(path: &Path) -> usize {
    path.as_os_str()
        .to_str()
        .map_or(path.as_os_str().len(), host_path::original_length)
}

/// IDA: 0040DFB2..0040E186 path selection: LocalAppData + "\Oblivion\" -> 00B3F178, Documents +
/// "\My Games\" + "Oblivion\" -> 00B3F280, both ".\" when the bootstrap value is 0, selected INI =
/// 00B3F280 + "Oblivion.ini". Completion: 100%.
#[must_use]
pub fn plan_startup_paths(roots: &PathRoots, use_my_games: bool) -> GamePaths {
    assert!(
        !roots.working.as_os_str().is_empty(),
        "Missing Oblivion startup working directory"
    );
    let settings_directory = if use_my_games {
        folder_root(&roots.documents).join("My Games/Oblivion")
    } else {
        roots.working.clone()
    };
    let local_data_directory = if use_my_games {
        folder_root(&roots.local_app_data).join("Oblivion")
    } else {
        roots.working.clone()
    };
    GamePaths {
        use_my_games,
        bootstrap_ini: roots.working.join("Oblivion.ini"),
        selected_ini: settings_directory.join("Oblivion.ini"),
        saves_directory: settings_directory.join("Saves"),
        settings_directory,
        local_data_directory,
    }
}

/// IDA: 0040DFB2 sprintf(".\\%s", "Oblivion.ini") and 0040DFCE
/// GetPrivateProfileIntA("General", "bUseMyGamesDirectory", 1, ..) != 0. Completion: 100%.
#[must_use]
pub fn use_my_games_from_bootstrap(bootstrap_ini: &Path) -> bool {
    profile::read_int(bootstrap_ini, b"General", b"bUseMyGamesDirectory", 1) != 0
}

/// IDA: N/A the folder sources of 0040DFF9 / 0040E071 on Linux: `OBLIVION_DOCUMENTS` and
/// `OBLIVION_LOCAL_APP_DATA`, else the user's Oblivion Wine prefix (`WINEPREFIX`, or
/// `~/.wine-oblivion`), so the host shares its existing INI and saves (user policy, 2026-09-24).
#[must_use]
pub fn host_path_roots(working: PathBuf) -> PathRoots {
    let user_directory = || -> Option<PathBuf> {
        let prefix = std::env::var_os("WINEPREFIX")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".wine-oblivion"))
            })?;
        let user = std::env::var_os("USER")?;
        Some(prefix.join("drive_c/users").join(user))
    };
    let folder = |variable: &str, relative: &str| {
        std::env::var_os(variable)
            .map(PathBuf::from)
            .or_else(|| user_directory().map(|directory| directory.join(relative)))
            .unwrap_or_default()
    };
    PathRoots {
        working,
        documents: folder("OBLIVION_DOCUMENTS", "Documents"),
        local_app_data: folder("OBLIVION_LOCAL_APP_DATA", "AppData/Local"),
    }
}

// IDA: N/A a helper, with no address of its own.
fn create_directory_failed(directory: &Path) -> bool {
    // CreateDirectoryA is one level only, and an existing directory is not a failure here.
    match std::fs::create_dir(directory) {
        Ok(()) => false,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => {
            emit(
                Level::Warning,
                &format!("Could not create startup directory {directory:?}: {error}"),
            );
            true
        }
    }
}

/// IDA: 0040E043 / 0040E0B8 / 0040E0F3 CreateDirectoryA in that order with results ignored, 0040E1B6
/// `_access(selected INI, 0)`, and 0040E221 CopyFileA("Oblivion_default.ini", selected, TRUE) with
/// the result ignored. The user's existing INI is never overwritten. Completion: 100%: failures are
/// logged and returned, so startup continues as in the original.
pub fn prepare_startup_paths(paths: &GamePaths) -> StartupPathPreparation {
    let mut result = StartupPathPreparation::default();
    if paths.use_my_games {
        result.directory_failures[0] = create_directory_failed(&paths.local_data_directory);
        let my_games = paths.settings_directory.parent().unwrap_or(Path::new("/"));
        result.directory_failures[1] = create_directory_failed(my_games);
        result.directory_failures[2] = create_directory_failed(&paths.settings_directory);
    }
    result.selected_ini_existed = paths.selected_ini.exists();
    if result.selected_ini_existed {
        return result;
    }
    let source = paths.bootstrap_ini.with_file_name("Oblivion_default.ini");
    result.default_copy_attempted = true;
    // CopyFileA's TRUE: fail when the target exists, which was just tested.
    match std::fs::copy(&source, &paths.selected_ini) {
        Ok(_) => result.default_copied = true,
        Err(error) => emit(
            Level::Warning,
            &format!(
                "Could not copy missing Oblivion INI from {source:?} to {:?}: {error}",
                paths.selected_ini
            ),
        ),
    }
    result
}

/// IDA: the length rule of 0040E021 / 0040E096 / 0040E0DA strcat and the byte copies. The original
/// overflows its buffers; the host keeps the full path and reports the overflow. Completion: 100%.
#[must_use]
pub fn original_path_buffer_overflow(paths: &GamePaths) -> OriginalPathBufferOverflow {
    const LIMIT: usize = 0x104;
    if !paths.use_my_games {
        return OriginalPathBufferOverflow::default();
    }
    // The original strings end with '\': host length + 1 separator.
    OriginalPathBufferOverflow {
        local_app_data: original_length(&paths.local_data_directory) + 1 >= LIMIT,
        my_games: original_length(&paths.settings_directory) + 1 >= LIMIT,
        selected_ini: original_length(&paths.selected_ini) >= LIMIT,
    }
}

/// IDA: 00404B1E..00404B73 the strcpy / strcat into the char[260] FileName. true = it would not fit
/// with its NUL. The host keeps the full path and continues. Completion: 100% of the length rule.
#[must_use]
pub fn original_plugins_path_overflow(paths: &GamePaths, file_name: &str) -> bool {
    let root = if paths.use_my_games {
        original_length(&paths.local_data_directory) + 1
    } else {
        2
    };
    root + host_path::original_length(file_name) + 1 > 0x104
}

/// IDA: 00404B00's FileName as the host opens it: local data + `file_name`. A leading ".\" in the
/// name (`.\Plugins.txt` at 00B02C88) stays a current-directory-style relative. Completion: 100%.
#[must_use]
pub fn plugins_list_path(paths: &GamePaths, file_name: &str) -> PathBuf {
    paths
        .local_data_directory
        .join(file_name.replace('\\', "/"))
}

/// IDA: 00404940 CopyProtection_FindDiscWithLauncher: its drive loop is skipped in this binary (00404987 jmp
/// 004049E5) and WinMain discards the result. User policy: no disc, true.
#[must_use]
pub fn disc_check_passed() -> bool {
    true
}
