//! The C++ `startup_scaffold` test's `verify_startup_paths`, `verify_startup_path_limits` and
//! `verify_non_ascii_startup_paths`, on Linux paths. Each C++ `\` joins with `/` here; the planned strings are the same
//! length, so the buffer limits carry over unchanged.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use oblivion_sim::tes::paths::{GamePaths, host_saves_directory};
use oblivion_sim::tes::winmain::path_setup::*;

/// `OBLIVION_SAVES_DIR` is process state: every test reading it holds this.
static SAVES_VARIABLE: Mutex<()> = Mutex::new(());

fn set_saves_override(value: Option<&Path>) {
    // SAFETY: the variable's readers in this binary hold `SAVES_VARIABLE`, and std's own environment lock covers the rest.
    unsafe {
        match value {
            Some(value) => std::env::set_var("OBLIVION_SAVES_DIR", value),
            None => std::env::remove_var("OBLIVION_SAVES_DIR"),
        }
    }
}

struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fresh_base(name: impl AsRef<std::ffi::OsStr>) -> Cleanup {
    let mut base = std::ffi::OsString::from(format!("recomp_{}_", std::process::id()));
    base.push(name);
    let base = std::env::temp_dir().join(base);
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    Cleanup(base)
}

fn roots_under(base: &Path, working: &str, documents: &str, local: &str) -> PathRoots {
    PathRoots {
        working: base.join(working),
        documents: base.join(documents),
        local_app_data: base.join(local),
    }
}

/// IDA: 0040DFCE and 0040E1C1..0040E221.
#[test]
fn verify_startup_paths() {
    let _guard = SAVES_VARIABLE.lock().unwrap();
    set_saves_override(None);
    let cleanup = fresh_base("startup_paths");
    let base = &cleanup.0;
    let roots = roots_under(base, "working", "documents", "local");
    std::fs::create_dir_all(&roots.working).unwrap();
    let missing = roots.working.join("Oblivion.ini");
    assert!(use_my_games_from_bootstrap(&missing));
    std::fs::write(&missing, "[General]\nbUseMyGamesDirectory=0\n").unwrap();
    assert!(!use_my_games_from_bootstrap(&missing), "the local opt-out");

    let portable = plan_startup_paths(&roots, false);
    assert_eq!(portable.selected_ini, missing);
    assert_eq!(portable.saves_directory, roots.working.join("Saves"));
    assert_eq!(portable.local_data_directory, roots.working);

    let user = plan_startup_paths(&roots, true);
    assert_eq!(
        user.selected_ini,
        roots.documents.join("My Games/Oblivion/Oblivion.ini")
    );
    assert_eq!(
        user.local_data_directory,
        roots.local_app_data.join("Oblivion")
    );
    assert_eq!(
        user.saves_directory,
        roots.documents.join("My Games/Oblivion/Saves")
    );
    assert_eq!(host_saves_directory(&user).unwrap(), user.saves_directory);

    std::fs::write(
        roots.working.join("Oblivion_default.ini"),
        "[General]\nStartupFixture=1\n",
    )
    .unwrap();
    // SHGetFolderPathA returns existing folders; CreateDirectoryA adds one level each.
    std::fs::create_dir_all(&roots.documents).unwrap();
    std::fs::create_dir_all(&roots.local_app_data).unwrap();
    let first = prepare_startup_paths(&user);
    assert!(user.settings_directory.is_dir());
    assert!(user.local_data_directory.is_dir());
    assert!(user.selected_ini.is_file());
    assert_eq!(first.directory_failures, [false; 3]);
    assert!(!first.selected_ini_existed && first.default_copy_attempted && first.default_copied);

    std::fs::write(&user.selected_ini, "existing user settings").unwrap();
    let second = prepare_startup_paths(&user);
    assert_eq!(second.directory_failures, [false; 3]);
    assert!(second.selected_ini_existed && !second.default_copy_attempted);
    assert_eq!(
        std::fs::read(&user.selected_ini).unwrap(),
        b"existing user settings"
    );

    std::fs::write(
        &user.selected_ini,
        "[General]\nSLocalSavePath=CustomSaves\\\n\
         [Display]\niSize W=1920\niSize H=1080\n\
         [GethitShader]\nfHitTexOffset=0.25\n\
         fBlockedTexOffset=0.125\nfBlurAmmount=0.75\n",
    )
    .unwrap();
    assert_eq!(
        host_saves_directory(&user).unwrap(),
        user.settings_directory.join("CustomSaves")
    );
    let override_directory = base.join("override-saves");
    set_saves_override(Some(&override_directory));
    assert_eq!(host_saves_directory(&user).unwrap(), override_directory);
    set_saves_override(None);
    std::fs::write(
        &user.selected_ini,
        "[General]\nSLocalSavePath=..\\Elsewhere\\\n",
    )
    .unwrap();
    assert!(
        host_saves_directory(&user).is_err(),
        "an escaping save path"
    );

    // 0040E0B8 / 0040E0F3 ignore CreateDirectoryA failures and 0040E221 ignores the failed copy.
    let bad_root = base.join("not-a-directory");
    std::fs::write(&bad_root, "").unwrap();
    let invalid = PathRoots {
        working: roots.working.clone(),
        documents: bad_root,
        local_app_data: roots.local_app_data.clone(),
    };
    let blocked = prepare_startup_paths(&plan_startup_paths(&invalid, true));
    assert_eq!(blocked.directory_failures, [false, true, true]);
    assert!(!blocked.selected_ini_existed);
    assert!(blocked.default_copy_attempted && !blocked.default_copied);

    // 0040E221 with no Oblivion_default.ini: the copy fails and is ignored.
    let bare = roots_under(base, "bare-working", "bare-documents", "bare-local");
    std::fs::create_dir_all(&bare.working).unwrap();
    let no_default = prepare_startup_paths(&plan_startup_paths(&bare, true));
    assert_eq!(no_default.directory_failures, [true; 3]);
    assert!(no_default.default_copy_attempted && !no_default.default_copied);
}

/// IDA: 0040DFF9 / 0040E071 ignored SHGetFolderPathA results and the char[0x104] buffers of 0040E021..0040E186. No files
/// are written. The C++ root is `C:\`, three bytes; the same three-byte text is one path component here.
#[test]
fn verify_startup_path_limits() {
    let working = PathBuf::from("C:\\Work");
    // A failed folder lookup leaves "" + "\Oblivion\" and "\My Games\Oblivion\".
    let failed = plan_startup_paths(
        &PathRoots {
            working: working.clone(),
            ..PathRoots::default()
        },
        true,
    );
    assert_eq!(failed.settings_directory, Path::new("/My Games/Oblivion"));
    assert_eq!(failed.local_data_directory, Path::new("/Oblivion"));
    assert_eq!(
        failed.selected_ini,
        Path::new("/My Games/Oblivion/Oblivion.ini")
    );

    let plan = |documents: usize, local: usize| {
        plan_startup_paths(
            &PathRoots {
                working: working.clone(),
                documents: PathBuf::from(format!("C:\\{}", "a".repeat(documents))),
                local_app_data: PathBuf::from(format!("C:\\{}", "b".repeat(local))),
            },
            true,
        )
    };
    // Documents "C:\" + n chars: INI string n + 34 bytes, My Games n + 22.
    let fits = original_path_buffer_overflow(&plan(225, 246));
    assert!(!fits.local_app_data && !fits.my_games && !fits.selected_ini);
    let ini_only = original_path_buffer_overflow(&plan(226, 246));
    assert!(!ini_only.local_app_data && !ini_only.my_games && ini_only.selected_ini);
    let all = original_path_buffer_overflow(&plan(238, 247));
    assert!(all.local_app_data && all.my_games && all.selected_ini);
    // The .\ roots of bUseMyGamesDirectory=0 cannot overflow.
    let portable = plan_startup_paths(
        &PathRoots {
            working,
            ..PathRoots::default()
        },
        false,
    );
    assert!(!original_path_buffer_overflow(&portable).selected_ini);
    assert!(disc_check_passed());
}

/// The limit counts the original's bytes: one per character of a UTF-8 path, where the C++ `ansi_length` asks the code
/// page, and the raw bytes of a path that is not UTF-8.
#[test]
fn path_limits_count_original_bytes() {
    let plan = |documents: PathBuf| {
        plan_startup_paths(
            &PathRoots {
                working: PathBuf::from("/w"),
                documents,
                local_app_data: PathBuf::from("/l"),
            },
            true,
        )
    };
    let wide = |count: usize| PathBuf::from(format!("C:\\{}", "я".repeat(count)));
    assert!(!original_path_buffer_overflow(&plan(wide(225))).selected_ini);
    assert!(original_path_buffer_overflow(&plan(wide(226))).selected_ini);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let raw = |count: usize| {
            let mut bytes = b"C:\\".to_vec();
            bytes.extend(std::iter::repeat_n(0xE9, count));
            PathBuf::from(std::ffi::OsString::from_vec(bytes))
        };
        assert!(!original_path_buffer_overflow(&plan(raw(225))).selected_ini);
        assert!(original_path_buffer_overflow(&plan(raw(226))).selected_ini);
    }
    let paths = GamePaths {
        use_my_games: true,
        local_data_directory: PathBuf::from("/l"),
        ..GamePaths::default()
    };
    // Root "/l" + 1 = 3, name + NUL: 256 characters fit in 0x104, whatever UTF-8 makes of them.
    assert!(!original_plugins_path_overflow(&paths, &"é".repeat(256)));
    assert!(original_plugins_path_overflow(&paths, &"é".repeat(257)));
}

fn check_startup_under(base: &Path, roots: &PathRoots) {
    std::fs::create_dir_all(&roots.working).unwrap();
    let bootstrap = roots.working.join("Oblivion.ini");
    std::fs::write(&bootstrap, "[General]\nbUseMyGamesDirectory=1\n").unwrap();
    assert!(use_my_games_from_bootstrap(&bootstrap), "{base:?}");
    std::fs::write(
        roots.working.join("Oblivion_default.ini"),
        "[General]\nSLocalSavePath=UnicodeSaves\\\n",
    )
    .unwrap();
    std::fs::create_dir_all(&roots.documents).unwrap();
    std::fs::create_dir_all(&roots.local_app_data).unwrap();
    let paths = plan_startup_paths(roots, true);
    let prepared = prepare_startup_paths(&paths);
    assert_eq!(prepared.directory_failures, [false; 3], "{base:?}");
    assert!(prepared.default_copied);
    assert!(paths.local_data_directory.is_dir());
    assert!(paths.selected_ini.is_file());
    assert_eq!(
        host_saves_directory(&paths).unwrap(),
        paths.settings_directory.join("UnicodeSaves")
    );
}

/// IDA: N/A host substitution (row 01-E): folders, the default INI and settings under roots outside any code page. The
/// C++ test's roots, then the same again under bytes that are not UTF-8, which Linux can hold and NTFS cannot.
#[test]
fn verify_non_ascii_startup_paths() {
    let _guard = SAVES_VARIABLE.lock().unwrap();
    set_saves_override(None);
    let cleanup = fresh_base("oblivion-recomp-Обливион-測試");
    let base = &cleanup.0;
    let roots = roots_under(base, "作業", "Докум", "本地");
    check_startup_under(base, &roots);

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let raw = std::ffi::OsStr::from_bytes(b"raw_\xE9\xFF");
        let raw_base = fresh_base(raw);
        let base = &raw_base.0;
        let roots = PathRoots {
            working: base.join(std::ffi::OsStr::from_bytes(b"w\xFE")),
            documents: base.join(std::ffi::OsStr::from_bytes(b"d\xFD")),
            local_app_data: base.join(std::ffi::OsStr::from_bytes(b"l\xFC")),
        };
        check_startup_under(base, &roots);
    }

    // A save path whose INI bytes are not UTF-8 lifts one to one.
    let paths = plan_startup_paths(&roots, true);
    std::fs::write(
        &paths.selected_ini,
        b"[General]\nSLocalSavePath=caf\xE9\\\n",
    )
    .unwrap();
    assert_eq!(
        host_saves_directory(&paths).unwrap(),
        paths.settings_directory.join("café")
    );
}
