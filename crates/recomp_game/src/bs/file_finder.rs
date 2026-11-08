//! FileFinder (00B33A04) and the BSFile open and exists routes.
//!
//! The singleton becomes a `Resource`, and the 00B27E4C / 00B27E50 callback slots become
//! `BsFileCallbacks`. The archive list (00B338E0) is passed explicitly, because it is a separate
//! resource. Completion: 95%; the viewer-strings slot 00431C10 is debug-only.

use std::path::{Path, PathBuf};

use bevy_ecs::prelude::*;
use recomp_core::crt::strnicmp;
use recomp_core::host_path;

use super::archive::ArchiveManager;
use super::file::{BsFile, NiFile, NiFileMode, OpenedFile};

const FILE_FINDER_PATH_BUFFER: usize = 260;

/// IDA: 00B27E4C / 00B27E50 slots. The image holds the NiFile defaults 00748110 / 00748170; 004316E0
/// installs 00431440 / 00431370, and nothing uninstalls them.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BsFileCallbacks {
    pub open: u32,
    pub exists: u32,
}

impl BsFileCallbacks {
    pub const NI_FILE_OPEN: u32 = 0x0074_8110;
    pub const NI_FILE_EXISTS: u32 = 0x0074_8170;
    pub const FILE_FINDER_OPEN: u32 = 0x0043_1440;
    pub const FILE_FINDER_EXISTS: u32 = 0x0043_1370;
}

impl Default for BsFileCallbacks {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            open: Self::NI_FILE_OPEN,
            exists: Self::NI_FILE_EXISTS,
        }
    }
}

/// The result of 00431020: status 0 none, 1 loose, 2 packed; the 260-byte resolved copy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindFileResult {
    pub status: i32,
    pub resolved: String,
}

/// IDA: 004316E0 FileFinder, with the embedded NiTArray of search paths at +0x04. The working root is
/// IDA: N/A: the host's capture of the process directory.
#[derive(Resource, Debug)]
pub struct FileFinder {
    working_directory: PathBuf,
    paths: Vec<String>,
}

impl FileFinder {
    /// IDA: 004316E0: every construction installs the BSFile callbacks.
    pub fn new(working_directory: impl Into<PathBuf>, callbacks: &mut BsFileCallbacks) -> Self {
        callbacks.open = BsFileCallbacks::FILE_FINDER_OPEN;
        callbacks.exists = BsFileCallbacks::FILE_FINDER_EXISTS;
        Self {
            working_directory: working_directory.into(),
            paths: Vec::new(),
        }
    }

    /// IDA: 00431B60 AddSearchPath: a copy with one '\' appended when missing; duplicates are kept.
    /// Completion: 100%.
    pub fn add(&mut self, path: &str) {
        let mut copy = path.to_owned();
        if !copy.ends_with('\\') {
            copy.push('\\');
        }
        self.paths.push(copy);
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    /// IDA: 00431020 FileFinder_FindFile(path, out, flags, mask): mask -1 -> 0xFFFF; path[1] == ':'
    /// adds flags 5; ".\" adds 4 and is skipped; one leading '\' is skipped. On failure the output is
    /// "". Inputs of 260+ characters (a stack overrun in the original) return status 0. Completion:
    /// 100%.
    pub fn find_file(
        &self,
        raw_path: &str,
        mut flags: u32,
        type_mask: i32,
        archives: Option<&mut ArchiveManager>,
    ) -> FindFileResult {
        let path = raw_path.split('\0').next().unwrap_or(raw_path);
        let mask = if type_mask == -1 {
            0xFFFF
        } else {
            type_mask as u32
        };
        let bytes = path.as_bytes();
        let mut start = 0;
        // path[1] of the original's bytes is the second character of the lifted string.
        let second = path.chars().nth(1).unwrap_or('\0');
        if second == ':' {
            flags |= 5;
        } else if bytes.first() == Some(&b'.') && second == '\\' {
            flags |= 4;
            start = 2;
        }
        if bytes.get(start) == Some(&b'\\') {
            start += 1;
        }
        let mut local = path[start.min(path.len())..].to_owned();
        if host_path::original_length(&local) >= FILE_FINDER_PATH_BUFFER {
            return FindFileResult::default();
        }
        let status = self.search_for_file(&mut local, flags as u8, mask, archives);
        if status == 0 {
            return FindFileResult::default();
        }
        FindFileResult {
            status,
            resolved: local,
        }
    }

    /// IDA: 00431800 FileFinder_SearchForFile(path, u8 flags, mask): flag 1 clear and 0042EBA0 -> 2;
    /// flag 2 clear and `_access` -> 1; flag 4 -> 0; then each root in order, skipped when path already
    /// starts with it, root + path rewritten into `path` when `_access` succeeds -> 1; else 0.
    /// Completion: 100%.
    pub fn search_for_file(
        &self,
        path: &mut String,
        flags: u8,
        type_mask: u32,
        archives: Option<&mut ArchiveManager>,
    ) -> i32 {
        if flags & 1 == 0
            && !path.is_empty()
            && let Some(archives) = archives
            && archives.find_packed(path, type_mask).is_some()
        {
            return 2;
        }
        if flags & 2 == 0 && host_path::access(&self.working_directory, path) {
            return 1;
        }
        if flags & 4 != 0 {
            return 0;
        }
        for root in &self.paths {
            if strnicmp(path.as_bytes(), root.as_bytes(), root.len()) == 0 {
                continue;
            }
            let candidate = format!("{root}{path}");
            if host_path::original_length(&candidate) < FILE_FINDER_PATH_BUFFER
                && host_path::access(&self.working_directory, &candidate)
            {
                *path = candidate;
                return 1;
            }
        }
        0
    }
}

/// IDA: 00430EF0 BSFile_OpenCore(path, mode, options, mask): mode 1 is a BSFile on the unresolved path
/// (which opens at once); otherwise FindFile: status 0 -> null, 2 -> 0042E8E0, else a BSFile on the
/// resolved path. Completion: 100%.
pub fn bs_file_open_core(
    finder: &FileFinder,
    mut archives: Option<&mut ArchiveManager>,
    path: &str,
    mode: i32,
    options: u32,
    type_mask: u32,
) -> Option<OpenedFile> {
    let source = path.split('\0').next().unwrap_or(path);
    if mode == 1 {
        return Some(OpenedFile::Loose(BsFile::new(
            finder.working_directory(),
            source,
            NiFileMode::WriteOnly,
            options,
        )));
    }
    let found = finder.find_file(source, 0, -1, archives.as_deref_mut());
    match found.status {
        0 => None,
        2 => archives?.open_packed(&found.resolved, options as i32, type_mask),
        _ => {
            let mode = if mode == 2 {
                NiFileMode::AppendOnly
            } else {
                NiFileMode::ReadOnly
            };
            Some(OpenedFile::Loose(BsFile::new(
                finder.working_directory(),
                &found.resolved,
                mode,
                options,
            )))
        }
    }
}

// IDA: N/A a helper, with no address of its own.
fn ni_mode(mode: i32) -> NiFileMode {
    match mode {
        1 => NiFileMode::WriteOnly,
        2 => NiFileMode::AppendOnly,
        _ => NiFileMode::ReadOnly,
    }
}

/// IDA: the open through the 00B27E4C slot (thunk 00748100): 00748110 an NiFile on the unresolved
/// path (returned even when it failed to open), or 00431440 -> 00430EF0(.., 0xFFFF), null without a
/// singleton. `default_root` stands in for the process directory. Completion: 100%.
pub fn bs_file_open(
    callbacks: &BsFileCallbacks,
    default_root: &Path,
    finder: Option<&FileFinder>,
    archives: Option<&mut ArchiveManager>,
    path: &str,
    mode: i32,
    options: u32,
) -> Option<OpenedFile> {
    if callbacks.open == BsFileCallbacks::NI_FILE_OPEN {
        return Some(OpenedFile::Ni(NiFile::new(
            default_root,
            path,
            ni_mode(mode),
            options,
        )));
    }
    bs_file_open_core(finder?, archives, path, mode, options, 0xFFFF)
}

/// IDA: 00431690 / 00431130 BSFile_OpenAsBSFile(Masked): 00430EF0, then `__RTDynamicCast` to BSFile.
/// Completion: 100%.
pub fn bs_file_open_as_bs_file(
    finder: Option<&FileFinder>,
    archives: Option<&mut ArchiveManager>,
    path: &str,
    mode: i32,
    options: u32,
    mask: u32,
) -> Option<OpenedFile> {
    bs_file_open_core(finder?, archives, path, mode, options, mask).filter(OpenedFile::is_bs_file)
}

/// IDA: the exists through the 00B27E50 slot: 00748170 NiFile good, or 00431370: mode 0 -> FindFile
/// != 0 with a singleton; another mode -> NiFile good. Modes 1 and 2 open "wb" / "ab", so they create
/// or truncate; all seven callers in the executable pass mode 0. Completion: 100%.
pub fn bs_file_exists(
    callbacks: &BsFileCallbacks,
    default_root: &Path,
    finder: Option<&FileFinder>,
    archives: Option<&mut ArchiveManager>,
    path: &str,
    mode: i32,
) -> bool {
    let source = path.split('\0').next().unwrap_or(path);
    let probe = |root: &Path| {
        use super::file::GameFile;
        NiFile::new(root, source, ni_mode(mode), 0).good()
    };
    if callbacks.exists == BsFileCallbacks::NI_FILE_EXISTS {
        return probe(default_root);
    }
    if mode != 0 {
        return probe(finder.map_or(default_root, FileFinder::working_directory));
    }
    finder.is_some_and(|finder| finder.find_file(source, 0, -1, archives).status != 0)
}
