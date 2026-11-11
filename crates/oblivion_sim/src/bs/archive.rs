//! BSA v103 archives and the archive manager (B338E0 list, B338E4 cache, B338E8 first-by-type).
//!
//! Names are lifted game strings (`host_path`): each `char` is one original byte, so lengths count characters and the
//! hashes run over `host_path::original_bytes`. The shipped data is ASCII.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, MutexGuard};

use bevy_ecs::prelude::*;
use oblivion_core::crt::{TextModeFile, stricmp};
use oblivion_core::hash::bsa_name_hash;
use oblivion_core::host_path;
use oblivion_core::logging::{Level, emit};

use super::file::{
    ARCHIVE_FILE_BUFFER_LIMIT_128K, ARCHIVE_FILE_BUFFER_LIMIT_IMAGE, ArchiveFile, ArchiveShared,
    BsFile, CompressedArchiveFile, GameFile, NiFileMode, OpenedFile, SEEK_SET,
};
use super::file_finder::{FileFinder, bs_file_open_core};
use crate::gamebryo::file::SEEK_CUR;

const BSA_MAGIC: u32 = 0x0041_5342;
const BSA_MAX_VERSION: u32 = 0x67;
const SPLIT_COMPONENT_LIMIT: usize = 255;
const LINE_BUFFER: u32 = 260;
/// IDA: 0042F55D Archive(path, 0x40000, ..) stream buffer; 0042F473 0x2800.
const ARCHIVE_STREAM_BUFFER: u32 = 0x4_0000;
const ARCHIVE_STREAM_BUFFER_AFTER_LOAD: u32 = 0x2800;

/// IDA: the legacy CRT `_splitpath` 006FA1B0, 006FA2D0, 0042CE40 and 0042D000 use: drive = "X:" when
/// path[1] == ':', the directory through the last '\' or '/', the extension from the last '.' after
/// it; components cut at 255 characters. Completion: 100% below 260 characters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SplitPath {
    pub drive: String,
    pub directory: String,
    pub file_name: String,
    pub extension: String,
}

// IDA: N/A a helper, with no address of its own.
fn cut(text: &str, limit: usize) -> &str {
    text.char_indices()
        .nth(limit)
        .map_or(text, |(end, _)| &text[..end])
}

/// IDA: N/A the last `count` original bytes of a lifted string, or `None` when it is shorter.
fn last_characters(text: &str, count: usize) -> Option<&str> {
    let length = host_path::original_length(text);
    if length < count {
        return None;
    }
    Some(cut_from(text, length - count))
}

// IDA: N/A a helper, with no address of its own.
fn cut_from(text: &str, skip: usize) -> &str {
    text.char_indices()
        .nth(skip)
        .map_or("", |(start, _)| &text[start..])
}

/// IDA: N/A a helper, with no address of its own.
#[must_use]
pub fn split_path(raw_path: &str) -> SplitPath {
    let path = raw_path.split('\0').next().unwrap_or(raw_path);
    let bytes = path.as_bytes();
    let mut result = SplitPath::default();
    let mut start = 0;
    // path[1] of the original's bytes is the second character here.
    let mut characters = path.char_indices();
    if let (Some(_), Some((colon, ':'))) = (characters.next(), characters.next()) {
        start = colon + 1;
        result.drive = path[..start].to_owned();
    }
    let mut last_slash = None;
    let mut dot = None;
    for (index, byte) in bytes.iter().enumerate().skip(start) {
        match byte {
            b'/' | b'\\' => last_slash = Some(index + 1),
            b'.' => dot = Some(index),
            _ => {}
        }
    }
    if let Some(slash) = last_slash {
        result.directory = cut(&path[start..slash], SPLIT_COMPONENT_LIMIT).to_owned();
        start = slash;
    }
    match dot {
        Some(dot) if dot >= start => {
            result.file_name = cut(&path[start..dot], SPLIT_COMPONENT_LIMIT).to_owned();
            result.extension = cut(&path[dot..], SPLIT_COMPONENT_LIMIT).to_owned();
        }
        _ => result.file_name = cut(&path[start..], SPLIT_COMPONENT_LIMIT).to_owned(),
    }
    result
}

/// IDA: 006FA221..006FA298 the shared folder rule: drive + dir, "." when empty, one trailing '\'
/// removed.
fn folder_text(split: &SplitPath) -> String {
    let mut folder = format!("{}{}", split.drive, split.directory);
    if folder.is_empty() {
        folder.push('.');
    }
    if folder.ends_with('\\') {
        folder.pop();
    }
    folder
}

// IDA: N/A a helper, with no address of its own.
fn file_name_hash(split: &SplitPath) -> u64 {
    bsa_name_hash(
        &host_path::original_bytes(&split.file_name),
        Some(&host_path::original_bytes(&split.extension)),
    )
}

// IDA: N/A a helper, with no address of its own.
fn folder_hash(split: &SplitPath) -> u64 {
    bsa_name_hash(&host_path::original_bytes(&folder_text(split)), None)
}

/// IDA: 006FA1B0's two hashes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BsaPathHashes {
    pub folder: u64,
    pub file: u64,
}

/// IDA: 006FA1B0(path, folderOut, fileOut). Completion: 100%.
#[must_use]
pub fn bsa_path_hashes(full_path: &str) -> BsaPathHashes {
    let split = split_path(full_path);
    BsaPathHashes {
        file: file_name_hash(&split),
        folder: folder_hash(&split),
    }
}

/// IDA: 006FA2D0 modes: 0 file, 1 folder, 2 raw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BsaHashMode {
    File,
    Folder,
    Raw,
}

/// IDA: 006FA2D0(out, path, mode). Completion: 100%.
#[must_use]
pub fn bsa_hash_path(full_path: &str, mode: BsaHashMode) -> u64 {
    if mode == BsaHashMode::Raw {
        return bsa_name_hash(&host_path::original_bytes(full_path), None);
    }
    let split = split_path(full_path);
    if mode == BsaHashMode::File {
        return file_name_hash(&split);
    }
    folder_hash(&split)
}

/// IDA: B04368 the 24 {char ext[4]; u32 types} entries.
#[derive(Clone, Debug)]
pub struct ArchiveTypeTable {
    entries: Vec<(&'static str, u32)>,
}

impl Default for ArchiveTypeTable {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            entries: vec![
                ("kf", 0x1),
                ("nif", 0x1),
                ("egm", 0x1),
                ("egt", 0x1),
                ("tri", 0x1),
                ("cmp", 0x1),
                ("lod", 0x1),
                ("dds", 0x2),
                ("tga", 0x2),
                ("bmp", 0x2),
                ("wav", 0x8),
                ("mp3", 0x10),
                ("lip", 0x10),
                ("xml", 0x4),
                ("spt", 0x40),
                ("fnt", 0x80),
                ("tex", 0x80),
                ("vso", 0x20),
                ("pso", 0x20),
                ("vsh", 0x20),
                ("psh", 0x20),
                ("lsl", 0x20),
                ("h", 0x20),
                ("", 0x0),
            ],
        }
    }
}

impl ArchiveTypeTable {
    /// IDA: 0042F610 selection sort: for i = 0..22 the smallest later entry is swapped into i.
    pub fn sort(&mut self) {
        for index in 0..23 {
            let mut smallest = index;
            for next in index + 1..self.entries.len() {
                if stricmp(
                    self.entries[next].0.as_bytes(),
                    self.entries[smallest].0.as_bytes(),
                ) < 0
                {
                    smallest = next;
                }
            }
            self.entries.swap(index, smallest);
        }
    }

    /// IDA: 0042BA90(last three characters): skip s[0] when '.', else two when s[1] == '.', then
    /// binary search (lo never becomes mid + 1); a miss returns 0x100. Completion: 100%.
    #[must_use]
    pub fn lookup(&self, raw: &str) -> u32 {
        let mut key = raw.as_bytes();
        if key.first() == Some(&b'.') {
            key = &key[1..];
        } else if key.get(1) == Some(&b'.') {
            key = &key[2..];
        }
        let mut low = 0;
        let mut high = self.entries.len();
        loop {
            let half = (high - low) >> 1;
            let middle = low + half;
            let compare = stricmp(key, self.entries[middle].0.as_bytes());
            if compare == 0 {
                return self.entries[middle].1;
            }
            if compare > 0 {
                low += half;
            } else {
                high = middle;
            }
            if half == 0 {
                return 0x100;
            }
        }
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn extensions(&self) -> Vec<&'static str> {
        self.entries.iter().map(|entry| entry.0).collect()
    }
}

/// IDA: B04428..B04460 the [Archive] settings, with their image defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveSettings {
    pub use_archives: bool,
    pub check_runtime_collisions: bool,
    pub retain_directory_strings: i32,
    pub retain_filename_strings: i32,
    pub retain_filename_offsets: i32,
    pub invalidation_file: Vec<u8>,
    pub archive_list: Vec<u8>,
    pub invalidate_older_files: bool,
}

impl Default for ArchiveSettings {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            use_archives: true,
            check_runtime_collisions: false,
            retain_directory_strings: 1,
            retain_filename_strings: 1,
            retain_filename_offsets: 1,
            invalidation_file: b"ArchiveInvalidation.txt".to_vec(),
            archive_list: b"Oblivion - Meshes.bsa, Oblivion - Textures - Compressed.bsa, \
                            Oblivion - Sounds.bsa, Oblivion - Voices1.bsa, \
                            Oblivion - Voices2.bsa, Oblivion - Misc.bsa"
                .to_vec(),
            invalidate_older_files: true,
        }
    }
}

/// IDA: per-folder 16-byte file records: u64 hash, u32 size +8, u32 offset +12. Size bit 31 = "older
/// check done" (0042C1D0); offset bit 31 compares with runtime flag 0x08; an offset of 0 marks an
/// invalidated record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BsaFileRecord {
    pub hash: u64,
    pub size: u32,
    pub offset: u32,
}

/// IDA: +0x178 folder records: u64 hash, u32 count, u32 offset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BsaFolderRecord {
    pub hash: u64,
    pub count: u32,
    pub offset: u32,
    pub files: Vec<BsaFileRecord>,
}

/// IDA: 0042D840 lists: B33930 file hashes (lines without a backslash), B33934 folder hashes. Null
/// until the first entry of a kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArchiveInvalidationLists {
    pub files: Option<Vec<u64>>,
    pub folders: Option<Vec<u64>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveLoadStatus {
    Loaded,
    /// BSFile +0x24 false: `_access` failed, or the lazy fopen failed.
    Missing,
    /// Magic is not "BSA\0" or the version exceeds 0x67: runtime flag 1.
    InvalidHeader,
    /// Host guard: a table larger than the file could hold.
    UnsupportedTableSize,
}

/// IDA: 0042E070's packed selection: offset & 0x7FFFFFFF, size & 0x3FFFFFFF, and compressed =
/// ((size >> 30) ^ (archive flags >> 2)) & 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackedFileView {
    pub folder: u32,
    pub file: u32,
    pub offset: u32,
    pub size: u32,
    pub compressed: bool,
}

// IDA: N/A a helper, with no address of its own.
fn dword(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

// IDA: N/A a helper, with no address of its own.
fn qword(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes"))
}

// IDA: N/A a helper, with no address of its own.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

// IDA: N/A a helper, with no address of its own.
fn block_string(block: &[u8], offset: u32) -> String {
    let start = (offset as usize).min(block.len());
    let rest = &block[start..];
    latin1(
        &rest[..rest
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(rest.len())],
    )
}

/// IDA: 0042BD70 the names-retained predicate: bInvalidateOlderFiles or bCheckRuntimeCollisions,
/// archive flag 0x80, then B04438 / B04440 / B04448 (value 1 tests archive flag bits 3, 4 and 5;
/// other values test != 0). Completion: 100%.
fn names_retained(settings: &ArchiveSettings, archive_flags: u32) -> bool {
    if settings.invalidate_older_files
        || settings.check_runtime_collisions
        || archive_flags & 0x80 != 0
    {
        return true;
    }
    let rule = |value: i32, bit: u32| {
        if value == 1 {
            (archive_flags >> bit) & 1 != 0
        } else {
            value != 0
        }
    };
    rule(settings.retain_directory_strings, 3)
        || rule(settings.retain_filename_strings, 4)
        || rule(settings.retain_filename_offsets, 5)
}

/// IDA: 0042C2D0 record validity. Completion: 100%.
fn record_valid(runtime_flags: u8, record: &BsaFileRecord, ignore_lazy_flag: bool) -> bool {
    let lazy = u32::from((runtime_flags >> 3) & 1);
    (ignore_lazy_flag || lazy == record.offset >> 31) && record.offset & 0x7FFF_FFFF != 0
}

// IDA: N/A a helper, with no address of its own.
fn stat_mtime(working: &Path, relative: &str) -> Option<i64> {
    let modified = std::fs::metadata(host_path::resolve(working, relative))
        .ok()?
        .modified()
        .ok()?;
    Some(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64,
    )
}

/// IDA: 0042C1D0 the older-loose-file check: with bInvalidateOlderFiles and size bit 31 clear, a
/// "Data\" + path copy newer than the archive clears the offset (keeping bit 31) and rejects the
/// record, leaving size bit 31 clear; otherwise size bit 31 is set. A path too long for the 260-byte
/// buffer counts as a stat failure. Completion: 100%.
fn invalidate_if_older(
    record: &mut BsaFileRecord,
    path: Option<&str>,
    archive_mtime: i64,
    settings: &ArchiveSettings,
    working: &Path,
) -> bool {
    if !settings.invalidate_older_files || record.size & 0x8000_0000 != 0 {
        return false;
    }
    if let Some(path) = path {
        let loose = format!("Data\\{path}");
        if (loose.len() as u32) < LINE_BUFFER
            && stat_mtime(working, &loose).is_some_and(|mtime| mtime > archive_mtime)
        {
            record.offset &= 0x8000_0000;
            return true;
        }
    }
    record.size |= 0x8000_0000;
    false
}

/// IDA: 0042EE80 Archive (0x280), on its BSFile base.
#[derive(Debug)]
pub struct Archive {
    shared: Arc<ArchiveShared>,
    status: ArchiveLoadStatus,
    magic: u32,
    version: u32,
    archive_flags_160: u32,
    folder_count_164: u32,
    file_count_168: u32,
    file_flags_174: u32,
    folders_178: Vec<BsaFolderRecord>,
    modification_time_180: i64,
    file_names_offset_188: u32,
    last_folder_18c: u32,
    last_file_190: u32,
    runtime_flags_194: u8,
    folder_names_198: Vec<u8>,
    folder_name_offsets_19c: Vec<u32>,
    file_names_1a0: Vec<u8>,
    file_name_offsets_1a4: Vec<Vec<u32>>,
    short_read: bool,
    slow_invalidation_unsupported: bool,
    invalidated_files: u32,
}

impl Archive {
    // IDA: N/A an accessor.
    fn stream(&self) -> MutexGuard<'_, BsFile> {
        self.shared.stream.lock().expect("the archive lock")
    }

    /// IDA: 0042EE80 after 00430970 BSFile(path, mode 0): the 36-byte header over the 006FA180
    /// defaults; then, when not lazy, folder records, per-folder u8-length names and file records,
    /// and the file-name block with its offset tables; the mtime; 0042E2D0 when
    /// bInvalidateOlderFiles. Short reads keep zeroed or previous values. Completion: 95%: the a5
    /// registration 0042EDF0 is not ported (startup passes 0).
    #[must_use]
    pub fn open(
        working: &Path,
        relative_path: &str,
        lazy: bool,
        settings: &ArchiveSettings,
        lists: &ArchiveInvalidationLists,
        buffer_limit: Arc<AtomicU32>,
    ) -> Self {
        let mut reader = BsFile::new(
            working,
            relative_path,
            NiFileMode::ReadOnly,
            ARCHIVE_STREAM_BUFFER,
        );
        let mut archive = Self {
            shared: Arc::new(ArchiveShared::new(
                BsFile::new(working, "", NiFileMode::ReadOnly, 0),
                buffer_limit.clone(),
            )),
            status: ArchiveLoadStatus::Missing,
            magic: BSA_MAGIC,
            version: BSA_MAX_VERSION,
            archive_flags_160: 0,
            folder_count_164: 0,
            file_count_168: 0,
            file_flags_174: 0,
            folders_178: Vec::new(),
            modification_time_180: 0,
            file_names_offset_188: 0,
            last_folder_18c: u32::MAX,
            last_file_190: u32::MAX,
            runtime_flags_194: 0,
            folder_names_198: Vec::new(),
            folder_name_offsets_19c: Vec::new(),
            file_names_1a0: Vec::new(),
            file_name_offsets_1a4: Vec::new(),
            short_read: false,
            slow_invalidation_unsupported: false,
            invalidated_files: 0,
        };
        if reader.good() {
            archive.load(&mut reader, working, lazy, settings, lists);
        }
        archive.shared = Arc::new(ArchiveShared::new(reader, buffer_limit));
        archive
    }

    // IDA: N/A no address of its own; its comments cite 0042EF90.
    fn load(
        &mut self,
        reader: &mut BsFile,
        working: &Path,
        lazy: bool,
        settings: &ArchiveSettings,
        lists: &ArchiveInvalidationLists,
    ) {
        if lazy {
            self.runtime_flags_194 = 0x08;
        }
        // 0042EF90: the header over the 006FA180 defaults.
        let mut header = [0u8; 36];
        header[0..4].copy_from_slice(&BSA_MAGIC.to_le_bytes());
        header[4..8].copy_from_slice(&BSA_MAX_VERSION.to_le_bytes());
        header[8..12].copy_from_slice(&36u32.to_le_bytes());
        let mut loaded = [0u8; 36];
        let header_bytes = reader.read(&mut loaded);
        header[..header_bytes].copy_from_slice(&loaded[..header_bytes]);
        self.short_read = header_bytes < header.len();
        self.magic = dword(&header, 0);
        self.version = dword(&header, 4);
        self.archive_flags_160 = dword(&header, 12);
        self.folder_count_164 = dword(&header, 16);
        self.file_count_168 = dword(&header, 20);
        let folder_name_length = dword(&header, 24);
        let file_name_length = dword(&header, 28);
        self.file_flags_174 = dword(&header, 32);
        if self.magic != BSA_MAGIC || self.version > BSA_MAX_VERSION {
            // 0042F481: invalid header; no mtime, no 004303F0.
            self.runtime_flags_194 |= 0x01;
            self.status = ArchiveLoadStatus::InvalidHeader;
            return;
        }
        let file_size = if reader.good() {
            std::fs::metadata(host_path::resolve(working, reader.path()))
                .map_or(0, |data| data.len())
        } else {
            0
        };
        let too_large = |entries: u64, width: u64| entries * width > file_size;
        if self.runtime_flags_194 & 0x08 == 0 {
            if too_large(u64::from(self.folder_count_164), 16)
                || too_large(u64::from(folder_name_length), 1)
                || too_large(u64::from(file_name_length), 1)
            {
                self.status = ArchiveLoadStatus::UnsupportedTableSize;
                return;
            }
            emit(
                Level::Verbose,
                &format!("Loading archive {}", reader.path()),
            );
            // 0042EFD0..0042F030: folder records.
            let count = self.folder_count_164 as usize;
            let mut raw = vec![0u8; count * 16];
            let folder_bytes = reader.read(&mut raw);
            self.short_read |= folder_bytes < raw.len();
            self.folders_178 = raw
                .as_chunks::<16>()
                .0
                .iter()
                .map(|record| BsaFolderRecord {
                    hash: qword(record, 0),
                    count: dword(record, 8),
                    offset: dword(record, 12),
                    files: Vec::new(),
                })
                .collect();
            // 0042F03E / 0042F095: name tables kept under 0042BD70.
            let keep = names_retained(settings, self.archive_flags_160);
            if self.archive_flags_160 & 0x01 != 0 && keep {
                self.runtime_flags_194 |= 0x10;
                self.folder_names_198 = vec![0; folder_name_length as usize];
                self.folder_name_offsets_19c = vec![0; count];
            }
            if self.archive_flags_160 & 0x02 != 0 && keep {
                self.runtime_flags_194 |= 0x20;
                self.file_names_1a0 = vec![0; file_name_length as usize];
                self.file_name_offsets_1a4 = vec![Vec::new(); count];
            }
            // 0042F0E0..0042F20F: per-folder names and file records. The u8 length keeps its
            // previous value after a short read.
            let mut name_length = 0u8;
            let mut name_cursor = 0u32;
            for index in 0..count {
                if self.archive_flags_160 & 0x01 != 0 {
                    let mut byte = [name_length];
                    if reader.read(&mut byte) == 1 {
                        name_length = byte[0];
                    } else {
                        self.short_read = true;
                    }
                    let length = usize::from(name_length);
                    if keep {
                        // The original writes past a too-small +0x198 block; the host grows its copy.
                        let end = name_cursor as usize + length;
                        if end > self.folder_names_198.len() {
                            self.folder_names_198.resize(end, 0);
                        }
                        let range = name_cursor as usize..end;
                        self.short_read |= reader.read(&mut self.folder_names_198[range]) < length;
                        self.folder_name_offsets_19c[index] = name_cursor;
                        name_cursor += u32::from(name_length);
                    } else {
                        let mut discard = [0u8; 256];
                        self.short_read |= reader.read(&mut discard[..length]) < length;
                    }
                }
                let folder_count = self.folders_178[index].count;
                if too_large(u64::from(folder_count), 16) {
                    self.status = ArchiveLoadStatus::UnsupportedTableSize;
                    return;
                }
                let mut records = vec![0u8; folder_count as usize * 16];
                self.short_read |= reader.read(&mut records) < records.len();
                self.folders_178[index].files = records
                    .as_chunks::<16>()
                    .0
                    .iter()
                    .map(|record| BsaFileRecord {
                        hash: qword(record, 0),
                        size: dword(record, 8),
                        offset: dword(record, 12),
                    })
                    .collect();
            }
            // 0042F221..0042F30F: the file-name block (+0x188 = bytes read so far).
            if self.archive_flags_160 & 0x02 != 0 {
                self.file_names_offset_188 = reader.effective_position();
                if keep {
                    let wanted = self.file_names_1a0.len();
                    self.short_read |= reader.read(&mut self.file_names_1a0) < wanted;
                    let mut cursor = 0u32;
                    for index in 0..count {
                        let files = self.folders_178[index].count as usize;
                        let offsets = &mut self.file_name_offsets_1a4[index];
                        offsets.clear();
                        for _ in 0..files {
                            offsets.push(cursor);
                            cursor += block_string(&self.file_names_1a0, cursor).len() as u32 + 1;
                        }
                    }
                }
            }
            emit(
                Level::Verbose,
                &format!(
                    "Finished loading archive {} containing {} directories and {} files",
                    reader.path(),
                    self.folder_count_164,
                    self.file_count_168
                ),
            );
        }
        // 0042F36D: _stat64i32 on +0x3C (failure leaves stack garbage; host 0).
        match stat_mtime(working, reader.path()) {
            Some(mtime) => self.modification_time_180 = mtime,
            None => emit(
                Level::Warning,
                &format!("Could not find Archive {} to get filetime.", reader.path()),
            ),
        }
        if self.runtime_flags_194 & 0x08 == 0 && settings.invalidate_older_files {
            self.invalidated_files = self.invalidate(lists, working);
        }
        // 0042F473: the stream keeps a 0x2800 buffer, rewound to 0 (004303F0).
        reader.set_buffer_size(ARCHIVE_STREAM_BUFFER_AFTER_LOAD);
        self.status = if reader.good() {
            ArchiveLoadStatus::Loaded
        } else {
            ArchiveLoadStatus::Missing
        };
    }

    /// IDA: 0042E2D0 the invalidation pass. Completion: 90%.
    /// TODO: [ ] 0042E51F's FindFirstFileA time and the recursive 0042D2A0 "Data\" walk, for an
    /// archive without name strings. No shipped archive takes it.
    fn invalidate(&mut self, lists: &ArchiveInvalidationLists, working: &Path) -> u32 {
        if lists.folders.is_none() && lists.files.is_none() {
            return 0;
        }
        if self.archive_flags_160 & 0x03 != 0x03 {
            emit(
                Level::Warning,
                "bInvalidateOlderFiles is true in the INI file, but the archive doesn't have directory or file strings.",
            );
            self.slow_invalidation_unsupported = true;
            return 0;
        }
        let contains = |list: &Option<Vec<u64>>, hash: u64| {
            list.as_ref().is_some_and(|list| list.contains(&hash))
        };
        let mut invalidated = 0;
        for index in 0..self.folder_count_164 {
            let folder_hash = self.folders_178[index as usize].hash;
            if contains(&lists.folders, folder_hash) {
                let folder = &mut self.folders_178[index as usize];
                for record in &mut folder.files {
                    record.offset &= 0x8000_0000;
                }
                invalidated += folder.count;
                continue;
            }
            let Some(name) = self.folder_name(index) else {
                continue;
            };
            if !host_path::access(working, &format!("Data\\{name}")) {
                continue;
            }
            for record in &mut self.folders_178[index as usize].files {
                if contains(&lists.files, record.hash) {
                    record.offset &= 0x8000_0000;
                    invalidated += 1;
                }
            }
        }
        invalidated
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn status(&self) -> ArchiveLoadStatus {
        self.status
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn path(&self) -> String {
        self.stream().path().to_owned()
    }
    /// IDA: BSFile +0x24.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.stream().good()
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn stream_buffer_size(&self) -> u32 {
        self.stream().buffer_size()
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn stream_position(&self) -> u32 {
        self.stream().position()
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn stream_has_handle(&self) -> bool {
        self.stream().has_handle()
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn open_count(&self) -> i32 {
        self.shared.open_count()
    }
    /// IDA: +0x194: 0x01 invalid, 0x08 lazy, 0x10 folder names kept, 0x20 file names kept.
    #[must_use]
    pub const fn runtime_flags(&self) -> u8 {
        self.runtime_flags_194
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn archive_flags(&self) -> u32 {
        self.archive_flags_160
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn file_flags(&self) -> u32 {
        self.file_flags_174
    }
    /// IDA: +0x174's low word, read by 0042F610's nine type slots.
    #[must_use]
    pub const fn types(&self) -> u16 {
        self.file_flags_174 as u16
    }
    /// IDA: 0042F5D4 OpenArchive stores a nonzero type word.
    pub const fn set_types(&mut self, types: u16) {
        self.file_flags_174 = (self.file_flags_174 & 0xFFFF_0000) | types as u32;
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn folder_count(&self) -> u32 {
        self.folder_count_164
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn file_count(&self) -> u32 {
        self.file_count_168
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn modification_time(&self) -> i64 {
        self.modification_time_180
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn short_read(&self) -> bool {
        self.short_read
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn invalidated_files(&self) -> u32 {
        self.invalidated_files
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn folders(&self) -> &[BsaFolderRecord] {
        &self.folders_178
    }

    /// IDA: N/A which of +0x198, +0x19C, +0x1A0 and +0x1A4 are non-null, as P14-RUN-001's AR rows print them.
    #[must_use]
    pub fn name_blocks(&self) -> [bool; 4] {
        [
            !self.folder_names_198.is_empty(),
            !self.folder_name_offsets_19c.is_empty(),
            !self.file_names_1a0.is_empty(),
            !self.file_name_offsets_1a4.is_empty(),
        ]
    }

    /// IDA: 0042CAE0's retained branch.
    #[must_use]
    pub fn folder_name(&self, folder: u32) -> Option<String> {
        if self.runtime_flags_194 & 0x10 == 0 {
            return None;
        }
        let offset = *self.folder_name_offsets_19c.get(folder as usize)?;
        Some(block_string(&self.folder_names_198, offset))
    }

    /// IDA: 0042CC00's retained branch.
    #[must_use]
    pub fn file_name(&self, folder: u32, file: u32) -> Option<String> {
        if self.runtime_flags_194 & 0x20 == 0 {
            return None;
        }
        let offset = *self
            .file_name_offsets_1a4
            .get(folder as usize)?
            .get(file as usize)?;
        Some(block_string(&self.file_names_1a0, offset))
    }

    /// IDA: 0042CAE0 Archive_GetFolderName: the retained name; else null without archive flag 1; else
    /// seek past the records and the earlier folders and read the name into a new block. Completion:
    /// 100%.
    pub fn read_folder_name(&mut self, folder: u32) -> Option<String> {
        if self.runtime_flags_194 & 0x10 != 0 {
            return self.folder_name(folder);
        }
        if self.archive_flags_160 & 0x01 == 0 {
            return None;
        }
        let counts: Vec<u32> = self.folders_178[..folder as usize]
            .iter()
            .map(|entry| entry.count)
            .collect();
        let start = 16 * self.folder_count_164 + 36;
        let mut stream = self.shared.stream.lock().expect("the archive lock");
        stream.seek(start as i32, SEEK_SET);
        for count in counts {
            let mut length = [0];
            stream.read(&mut length);
            stream.seek((u32::from(length[0]) + 16 * count) as i32, SEEK_CUR);
        }
        let mut length = [0];
        stream.read(&mut length);
        let mut name = vec![0; usize::from(length[0])];
        stream.read(&mut name);
        drop(stream);
        self.folder_names_198 = name;
        Some(block_string(&self.folder_names_198, 0))
    }

    /// IDA: 0042CC00 Archive_GetFileName: the retained name; else null without archive flag 2; else a
    /// new 0x100 block, from the offset table (the iRetainFilenameOffsetTable rule) or by skipping
    /// every earlier name. Completion: 100%; a name of 0x100+ bytes overruns the block in the original
    /// (host stop).
    pub fn read_file_name(
        &mut self,
        folder: u32,
        file: u32,
        settings: &ArchiveSettings,
    ) -> Option<String> {
        if self.runtime_flags_194 & 0x20 != 0 {
            return self.file_name(folder, file);
        }
        if self.archive_flags_160 & 0x02 == 0 {
            return None;
        }
        // IDA: N/A no address of its own; its comments cite 0042CD0A.
        fn read_name(stream: &mut BsFile, keep: Option<&mut Vec<u8>>) {
            let mut keep = keep;
            let mut length = 0;
            loop {
                let mut byte = [0];
                stream.read(&mut byte);
                if let Some(block) = keep.as_deref_mut() {
                    assert!(
                        length < block.len(),
                        "0042CD0A: a 0x100+ byte file name overruns +0x1A0"
                    );
                    block[length] = byte[0];
                }
                length += 1;
                if byte[0] == 0 {
                    return;
                }
            }
        }
        let offsets = if settings.retain_filename_offsets == 1 {
            self.archive_flags_160 & 0x20 != 0
        } else {
            settings.retain_filename_offsets != 0
        };
        let mut block = vec![0; 0x100];
        let names_at = self.file_names_offset_188;
        let earlier: Vec<u32> = self.folders_178[..folder as usize]
            .iter()
            .map(|entry| entry.count)
            .collect();
        let table_offset = if offsets {
            Some(
                *self
                    .file_name_offsets_1a4
                    .get(folder as usize)
                    .and_then(|table| table.get(file as usize))
                    .expect("0042CCB0: the released +0x1A4 offset table is read"),
            )
        } else {
            None
        };
        let mut stream = self.shared.stream.lock().expect("the archive lock");
        if let Some(offset) = table_offset {
            stream.seek((names_at + offset) as i32, SEEK_SET);
        } else {
            stream.seek(names_at as i32, SEEK_SET);
            for count in earlier {
                for _ in 0..count {
                    read_name(&mut stream, None);
                }
            }
            for _ in 0..file {
                read_name(&mut stream, None);
            }
        }
        read_name(&mut stream, Some(&mut block));
        drop(stream);
        self.file_names_1a0 = block;
        Some(block_string(&self.file_names_1a0, 0))
    }

    /// IDA: the first block of 0042C970 per archive. Completion: 100%.
    pub fn release_folder_names(&mut self) {
        if self.runtime_flags_194 & 0x04 == 0 {
            self.folder_names_198 = Vec::new();
            self.folder_name_offsets_19c = Vec::new();
        }
        self.runtime_flags_194 &= !0x10;
    }

    /// IDA: 0042C0D0 Archive_ReleaseFileNames(keepOffsets). Completion: 100%.
    pub fn release_file_names(&mut self, keep_offsets: bool) {
        if self.runtime_flags_194 & 0x04 == 0 {
            self.file_names_1a0 = Vec::new();
            if !keep_offsets {
                self.file_name_offsets_1a4 = Vec::new();
            }
        }
        self.runtime_flags_194 &= !0x20;
    }

    /// IDA: 0042CE40 find a folder by hash: the +0x18C cache returns without the collision check; a
    /// binary search (lo never mid + 1); the bCheckRuntimeCollisions name compare. Completion: 100%.
    pub fn find_folder(
        &mut self,
        hash: u64,
        path: Option<&str>,
        settings: &ArchiveSettings,
    ) -> Option<u32> {
        if self.runtime_flags_194 & 0x01 != 0 {
            return None;
        }
        if self.last_folder_18c < self.folder_count_164
            && self.folders_178[self.last_folder_18c as usize].hash == hash
        {
            return Some(self.last_folder_18c);
        }
        self.last_file_190 = u32::MAX;
        if self.folder_count_164 == 0 {
            return None;
        }
        let mut low = 0;
        let mut high = self.folder_count_164;
        loop {
            let half = (high - low) >> 1;
            let middle = low + half;
            let record = self.folders_178[middle as usize].hash;
            if hash == record {
                self.last_folder_18c = middle;
                if let Some(path) = path.filter(|_| {
                    settings.check_runtime_collisions && self.archive_flags_160 & 1 != 0
                }) {
                    let directory = split_path(path).directory;
                    if let Some(name) = self.read_folder_name(middle)
                        && stricmp(name.as_bytes(), directory.as_bytes()) != 0
                    {
                        emit(
                            Level::Warning,
                            &format!("HashMap Collision between {name} and {directory}"),
                        );
                        return None;
                    }
                }
                return Some(middle);
            }
            if hash > record {
                low += half;
            } else {
                high = middle;
            }
            if half == 0 {
                return None;
            }
        }
    }

    /// IDA: 0042D000 find a file in a folder: the +0x190 cache (validity before hash, no collision
    /// check), 0042C2D0 validity, 0042C1D0 invalidation, the collision check. Completion: 100%.
    pub fn find_file(
        &mut self,
        folder: u32,
        hash: u64,
        path: Option<&str>,
        ignore_lazy_flag: bool,
        settings: &ArchiveSettings,
        working: &Path,
    ) -> Option<u32> {
        if self.runtime_flags_194 & 0x01 != 0 {
            return None;
        }
        let runtime_flags = self.runtime_flags_194;
        let mtime = self.modification_time_180;
        let count = self.folders_178[folder as usize].count;
        let records = &mut self.folders_178[folder as usize].files;
        if self.last_file_190 < count {
            let cached = &mut records[self.last_file_190 as usize];
            if record_valid(runtime_flags, cached, ignore_lazy_flag) && cached.hash == hash {
                if invalidate_if_older(cached, path, mtime, settings, working) {
                    return None;
                }
                return Some(self.last_file_190);
            }
        }
        if count == 0 {
            return None;
        }
        let mut low = 0;
        let mut high = count;
        loop {
            let half = (high - low) >> 1;
            let middle = low + half;
            let record = &mut records[middle as usize];
            if hash == record.hash {
                if !record_valid(runtime_flags, record, ignore_lazy_flag)
                    || invalidate_if_older(record, path, mtime, settings, working)
                {
                    return None;
                }
                self.last_file_190 = middle;
                if let Some(path) = path.filter(|_| {
                    settings.check_runtime_collisions && self.archive_flags_160 & 2 != 0
                }) {
                    let name_part = split_path(path).file_name;
                    if let Some(name) = self.file_name(folder, middle)
                        && stricmp(name.as_bytes(), name_part.as_bytes()) != 0
                    {
                        emit(
                            Level::Warning,
                            &format!("HashMap Collision between {name} and {name_part}"),
                        );
                        return None;
                    }
                }
                return Some(middle);
            }
            if hash > record.hash {
                low += half;
            } else {
                high = middle;
            }
            if half == 0 {
                return None;
            }
        }
    }

    /// IDA: 0042E020.
    pub fn find(
        &mut self,
        hashes: BsaPathHashes,
        path: &str,
        settings: &ArchiveSettings,
        working: &Path,
    ) -> Option<(u32, u32)> {
        let folder = self.find_folder(hashes.folder, Some(path), settings)?;
        let file = self.find_file(folder, hashes.file, Some(path), false, settings, working)?;
        Some((folder, file))
    }

    /// IDA: 0042E070's selection, without the open-count bracket.
    #[must_use]
    pub fn packed_view(&self, folder: u32, file: u32) -> Option<PackedFileView> {
        let record = self.folders_178[folder as usize].files[file as usize];
        if (u32::from(self.runtime_flags_194 >> 3) ^ (record.offset >> 31)) & 1 != 0 {
            return None;
        }
        let offset = record.offset & 0x7FFF_FFFF;
        if offset == 0 {
            return None;
        }
        Some(PackedFileView {
            folder,
            file,
            offset,
            size: record.size & 0x3FFF_FFFF,
            compressed: ((record.size >> 30) ^ (self.archive_flags_160 >> 2)) & 1 != 0,
        })
    }

    /// IDA: 0042E070 Archive_OpenPackedFile: a CompressedArchiveFile when the record's compression
    /// bit differs from archive flag 4, else an ArchiveFile. Completion: 100%.
    #[must_use]
    pub fn open_packed_file(
        &self,
        folder: u32,
        file: u32,
        options: i32,
        path: &str,
    ) -> Option<OpenedFile> {
        self.shared.acquire();
        let result = self
            .folders_178
            .get(folder as usize)
            .filter(|entry| (file as usize) < entry.files.len())
            .and_then(|_| self.packed_view(folder, file))
            .map(|view| {
                let owner = self.shared.clone();
                if view.compressed {
                    OpenedFile::Compressed(CompressedArchiveFile::new(
                        path,
                        owner,
                        view.offset,
                        view.size,
                        options,
                    ))
                } else {
                    OpenedFile::Packed(ArchiveFile::new(
                        path,
                        owner,
                        view.offset,
                        view.size,
                        options,
                    ))
                }
            });
        self.shared.release();
        result
    }
}

/// IDA: 0042EA73 / 0042E8F5: one leading '\' and a case-sensitive "Data\" or "data\" prefix are
/// skipped. Completion: 100%.
fn strip_packed_prefix(raw_path: &str) -> &str {
    let mut path = raw_path.split('\0').next().unwrap_or(raw_path);
    path = path.strip_prefix('\\').unwrap_or(path);
    path.strip_prefix("Data\\")
        .or_else(|| path.strip_prefix("data\\"))
        .unwrap_or(path)
}

/// IDA: 0042BC50 BSHash_Compare(pattern, record). Completion: 100%.
fn bsa_compare_hash(pattern: u64, record: u64) -> i32 {
    let byte = |value: u64, index: u32| ((value >> (8 * index)) & 0xFF) as u8;
    let class = |value: u64| {
        ((byte(value, 0) >> 6) & 2) + ((byte(value, 3) >> 5) & 4) + (byte(value, 1) >> 7)
    };
    let differs = i32::from(class(pattern) != class(record));
    if byte(pattern, 3) & 0x7F == b'*' {
        return differs;
    }
    let (pattern_high, record_high) = ((pattern >> 32) as u32, (record >> 32) as u32);
    if pattern_high < record_high {
        return -1;
    }
    if pattern_high > record_high || byte(pattern, 2) != byte(record, 2) {
        return 1;
    }
    differs
}

/// IDA: the 0042DB50 search loop over the sorted hash array. Completion: 100%.
fn sorted_hashes_contain(hashes: &[u64], key: u64) -> bool {
    let mut low = 0;
    let mut high = hashes.len();
    loop {
        let half = (high - low) >> 1;
        let value = hashes[low + half];
        if key < value {
            high = low + half;
        } else if key == value {
            return true;
        } else {
            low += half;
        }
        if half == 0 {
            return false;
        }
    }
}

// IDA: N/A a helper, with no address of its own.
fn source_prefix(source: &str) -> &str {
    source.rfind('\\').map_or("", |slash| &source[..=slash])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidationFileStatus {
    NotFound,
    OpenFailed,
    Read,
    NotBsFile,
}

/// IDA: N/A a `BSFileEntry*`'s identity: the record's archive list index, folder and file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BsaRecordKey {
    pub archive: usize,
    pub folder: u32,
    pub file: u32,
}

/// IDA: B338E0 list, B338E4 cache, B338E8 and B3390C by-type slots, B33930 / B33934 lists. Archives
/// are named by their list index.
/// IDA: N/A how a seam reaches 00B338E0 when something else may hold it at other moments: a host keeps one manager behind this, a call at a time; a test hands the manager itself.
pub trait ArchiveAccess {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// Runs `body` with the manager, or with `None` for a null g_ArchiveList.
    fn with_archives(&mut self, body: &mut dyn FnMut(Option<&mut ArchiveManager>));
}

impl ArchiveAccess for ArchiveManager {
    /// IDA: N/A `ArchiveManager`'s side of `ArchiveAccess`.
    fn with_archives(&mut self, body: &mut dyn FnMut(Option<&mut ArchiveManager>)) {
        body(Some(self));
    }
}

#[derive(Resource, Debug)]
pub struct ArchiveManager {
    working_directory: std::path::PathBuf,
    settings: ArchiveSettings,
    types: ArchiveTypeTable,
    archives: Vec<Archive>,
    first_by_type: [Option<usize>; 9],
    secondary_by_type: [Option<usize>; 9],
    cached: Option<usize>,
    invalidation: ArchiveInvalidationLists,
    requested_names: Vec<String>,
    expanded_list: String,
    buffer_limit: Arc<AtomicU32>,
}

impl ArchiveManager {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn new(
        working_directory: impl Into<std::path::PathBuf>,
        settings: ArchiveSettings,
    ) -> Self {
        Self {
            working_directory: working_directory.into(),
            settings,
            types: ArchiveTypeTable::default(),
            archives: Vec::new(),
            first_by_type: [None; 9],
            secondary_by_type: [None; 9],
            cached: None,
            invalidation: ArchiveInvalidationLists::default(),
            requested_names: Vec::new(),
            expanded_list: String::new(),
            buffer_limit: Arc::new(AtomicU32::new(ARCHIVE_FILE_BUFFER_LIMIT_IMAGE)),
        }
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn archives(&self) -> &[Archive] {
        &self.archives
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn first_by_type(&self) -> &[Option<usize>; 9] {
        &self.first_by_type
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn secondary_by_type(&self) -> &[Option<usize>; 9] {
        &self.secondary_by_type
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn cached(&self) -> Option<usize> {
        self.cached
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn invalidation_lists(&self) -> &ArchiveInvalidationLists {
        &self.invalidation
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn requested_names(&self) -> &[String] {
        &self.requested_names
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn expanded_archive_list(&self) -> &str {
        &self.expanded_list
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn settings(&self) -> &ArchiveSettings {
        &self.settings
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn type_table(&self) -> &ArchiveTypeTable {
        &self.types
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }
    /// IDA: 00B04360's setter.
    pub fn set_buffer_limit_128k(&self) {
        self.buffer_limit
            .store(ARCHIVE_FILE_BUFFER_LIMIT_128K, Ordering::Relaxed);
    }
    /// IDA: N/A 00B04360's value, for a test.
    #[must_use]
    pub fn buffer_limit(&self) -> u32 {
        self.buffer_limit.load(Ordering::Relaxed)
    }
    /// IDA: N/A a test reset to the image value.
    pub fn reset_buffer_limit(&self) {
        self.buffer_limit
            .store(ARCHIVE_FILE_BUFFER_LIMIT_IMAGE, Ordering::Relaxed);
    }

    /// IDA: 0042D840: file lines without a backslash, folder lines with one (one leading '\'
    /// skipped), through ReadLine(Str, 260, '\r') and a one-byte read. Completion: 100%.
    pub fn read_invalidation_file(&mut self, finder: &FileFinder) -> InvalidationFileStatus {
        let name = latin1(&self.settings.invalidation_file);
        // 0042D88B -> 00431690 -> 00430EF0(.., 0xFFFF).
        let Some(mut file) = bs_file_open_core(finder, Some(self), &name, 0, 0x2800, 0xFFFF) else {
            return InvalidationFileStatus::NotFound;
        };
        if !file.is_bs_file() {
            return InvalidationFileStatus::NotBsFile;
        }
        // 0042D8A6 vt+0x18 Open(0, 0).
        if let OpenedFile::Loose(loose) = &mut file
            && !loose.open()
        {
            return InvalidationFileStatus::OpenFailed;
        }
        loop {
            let (consumed, line) = file.read_line(LINE_BUFFER, 13);
            if consumed == 0 {
                break;
            }
            let mut skipped = [0];
            file.read(&mut skipped);
            let text = latin1(&line);
            if text.contains('\\') {
                let folder = text.strip_prefix('\\').unwrap_or(&text);
                let hash = bsa_hash_path(folder, BsaHashMode::Folder);
                self.invalidation
                    .folders
                    .get_or_insert_with(Vec::new)
                    .push(hash);
            } else {
                let hash = bsa_hash_path(&text, BsaHashMode::File);
                self.invalidation
                    .files
                    .get_or_insert_with(Vec::new)
                    .push(hash);
            }
        }
        InvalidationFileStatus::Read
    }

    /// IDA: 0042F4C0 ArchiveManager_OpenArchive(name, types, 0): FindFile(name, flags 1, -1), Archive
    /// construction, rejection of +0x24 false or runtime flag 1, the append and a nonzero type-word
    /// store. Returns the list index. Completion: 100% with a5 = 0.
    pub fn open_archive(&mut self, name: &str, types: u16, finder: &FileFinder) -> Option<usize> {
        if !self.settings.use_archives {
            return None;
        }
        let found = finder.find_file(name, 1, -1, None);
        if found.status == 0 {
            return None;
        }
        // 0042F538 FormHeap(0x280): failure -> 0042F57F returns null. Host substitution: the reservation stands for the
        // object's storage, which `Archive` does not need, so the null branch stays reachable.
        let mut storage = Vec::<u8>::new();
        if storage.try_reserve_exact(0x280).is_err() {
            return None;
        }
        let archive = Archive::open(
            &self.working_directory,
            &found.resolved,
            false,
            &self.settings,
            &self.invalidation,
            self.buffer_limit.clone(),
        );
        drop(storage);
        if !archive.exists()
            || archive.runtime_flags() & 0x01 != 0
            || archive.status() == ArchiveLoadStatus::UnsupportedTableSize
        {
            return None;
        }
        self.archives.push(archive);
        let index = self.archives.len() - 1;
        if types != 0 {
            self.archives[index].set_types(types);
        }
        Some(index)
    }

    /// IDA: 0042F610 ArchiveManager_InitializeFromSettings. `plugins_directory` holds Plugins.txt.
    /// False when any listed archive failed to open (WinMain ignores it). Completion: 100%.
    pub fn initialize_from_settings(
        &mut self,
        finder: &FileFinder,
        plugins_directory: &Path,
    ) -> bool {
        // 0042F62D memset(B338E8, 0, 0x48): both nine-slot lists.
        self.first_by_type = [None; 9];
        self.secondary_by_type = [None; 9];
        if !self.settings.use_archives {
            return true;
        }
        self.read_invalidation_file(finder);
        self.types.sort();
        let mut list = latin1(&self.settings.archive_list);
        // 0042F760..0042F92D: Plugins.txt ".esp" lines add "Data\<stem>*.bsa" matches. A failed fgets
        // keeps the previous bytes, whose ".esp" is already overwritten, so it matches nothing.
        if let Ok(bytes) = std::fs::read(plugins_directory.join("Plugins.txt")) {
            let mut stream = TextModeFile::from_bytes(bytes);
            let mut line: Vec<u8> = Vec::new();
            loop {
                if let Some(next) = stream.fgets(LINE_BUFFER as usize) {
                    line = next;
                }
                let first = line.first().copied().unwrap_or(0);
                if first != b'#'
                    && first != 0
                    && first != b'\n'
                    && let Some(esp) = line.windows(4).position(|window| window == b".esp")
                {
                    line.truncate(esp);
                    line.extend_from_slice(b"*.bsa");
                    let pattern = format!("Data\\{}", latin1(&line));
                    for name in host_path::find_files(&self.working_directory, &pattern) {
                        if list.len() >= 2 && list.as_bytes()[list.len() - 2] == b'\n' {
                            list.truncate(list.len() - 2);
                        }
                        list.push_str(", ");
                        list.push_str(&name);
                    }
                }
                if stream.feof() {
                    break;
                }
            }
        }
        self.expanded_list.clone_from(&list);
        // 0042F943 _mbstok(list, ","), leading ' ', '\t', '\n' skipped.
        let mut all_opened = true;
        for token in list.split(',').filter(|token| !token.is_empty()) {
            let name = token.trim_start_matches([' ', '\t', '\n']).to_owned();
            self.requested_names.push(name.clone());
            let Some(index) = self.open_archive(&name, 0, finder) else {
                all_opened = false;
                continue;
            };
            let types = self.archives[index].types();
            for (slot, entry) in self.first_by_type.iter_mut().enumerate() {
                if types & (1 << slot) != 0 && entry.is_none() {
                    *entry = Some(index);
                }
            }
        }
        all_opened
    }

    // IDA: N/A no address of its own; its comments cite 0042EA73.
    fn locate(&mut self, raw_path: &str, mut type_mask: u32) -> Option<(usize, u32, u32)> {
        // 0042EA73: no list object before the first OpenArchive append.
        if self.archives.is_empty() {
            return None;
        }
        let path = strip_packed_prefix(raw_path);
        // 0042EAB6: only the low word is replaced. The original reads before a short string; the host
        // uses 0x100.
        if type_mask == 0xFFFF {
            type_mask = last_characters(path, 3).map_or(0x100, |tail| self.types.lookup(tail));
        }
        let hashes = bsa_path_hashes(path);
        let cached = self.cached;
        if let Some(index) = cached {
            let archive = &mut self.archives[index];
            if type_mask & archive.file_flags() != 0
                && let Some((folder, file)) =
                    archive.find(hashes, path, &self.settings, &self.working_directory)
            {
                return Some((index, folder, file));
            }
        }
        for index in 0..self.archives.len() {
            if Some(index) == cached || type_mask & self.archives[index].file_flags() == 0 {
                continue;
            }
            let archive = &mut self.archives[index];
            if let Some((folder, file)) =
                archive.find(hashes, path, &self.settings, &self.working_directory)
            {
                self.cached = Some(index);
                return Some((index, folder, file));
            }
        }
        None
    }

    /// IDA: 0042DB10(type, folderHash, fileHash, path): the first archive of the type or, only when that slot is null,
    /// the secondary one; 0042D240 then runs Archive_FindFolder and Archive_FindFileInFolder(ignoreLazy = 1) with the
    /// path. The record's identity stands in for its pointer. Completion: 100%.
    pub fn file_record_by_type(
        &mut self,
        archive_type: u32,
        hashes: BsaPathHashes,
        path: &str,
    ) -> Option<BsaRecordKey> {
        let slot = archive_type as usize;
        let archive = (*self.first_by_type.get(slot)?).or(self.secondary_by_type[slot])?;
        let record = &mut self.archives[archive];
        let folder = record.find_folder(hashes.folder, Some(path), &self.settings)?;
        let file = record.find_file(
            folder,
            hashes.file,
            Some(path),
            true,
            &self.settings,
            &self.working_directory,
        )?;
        Some(BsaRecordKey {
            archive,
            folder,
            file,
        })
    }

    /// IDA: 0042DB10(type, folderHash, fileHash, 0), the call 0046D610 makes: the same as `file_record_by_type` with a null path, which makes Archive_FindFolder and Archive_FindFileInFolder go by hash alone. Completion: 100%.
    pub fn file_record_by_hashes(
        &mut self,
        archive_type: u32,
        folder_hash: u64,
        file_hash: u64,
    ) -> Option<BsaRecordKey> {
        let slot = archive_type as usize;
        let archive = (*self.first_by_type.get(slot)?).or(self.secondary_by_type[slot])?;
        let record = &mut self.archives[archive];
        let folder = record.find_folder(folder_hash, None, &self.settings)?;
        let file = record.find_file(
            folder,
            file_hash,
            None,
            true,
            &self.settings,
            &self.working_directory,
        )?;
        Some(BsaRecordKey {
            archive,
            folder,
            file,
        })
    }

    /// IDA: 0042EA60 packed lookup. Returns the archive's list index.
    pub fn find_packed(&mut self, path: &str, type_mask: u32) -> Option<usize> {
        self.locate(path, type_mask).map(|location| location.0)
    }

    /// IDA: 0042E8E0 ArchiveManager_OpenPackedFile(path, options, mask). Completion: 100%.
    pub fn open_packed(&mut self, path: &str, options: i32, type_mask: u32) -> Option<OpenedFile> {
        let (index, folder, file) = self.locate(path, type_mask)?;
        self.archives[index].open_packed_file(folder, file, options, strip_packed_prefix(path))
    }

    /// IDA: 00431460 FileFinder_FindSingleFile(path, source, mask, 0), 00431970's arm for a path with no '*' or '?': with
    /// bInvalidateOlderFiles a loose file (FindFirstFileA), else 0042EBA0's archive (0042E7E0 FindPackedArchive); found, the
    /// source through its last '\' plus the path's last component. An empty list is null. Completion: 100%.
    pub fn find_single_file(&mut self, path: &str, source: &str, type_mask: u32) -> Vec<String> {
        let loose = self.settings.invalidate_older_files
            && !host_path::find_files(&self.working_directory, path).is_empty();
        if !loose && self.locate(path, type_mask).is_none() {
            return Vec::new();
        }
        match (source.rfind('\\'), path.rfind('\\')) {
            (Some(prefix), Some(name)) => {
                vec![format!("{}{}", &source[..=prefix], &path[name + 1..])]
            }
            _ => Vec::new(),
        }
    }

    /// IDA: 00431970 FileFinder_FindMatchingFiles for a wildcard pattern, with 0042EC70: loose matches,
    /// then each archive's records through 0042DB50, each prepended. Returns the list head first.
    /// Completion: 100% of the wildcard path.
    pub fn find_matching_files(
        &mut self,
        pattern: &str,
        source: &str,
        type_mask: u32,
    ) -> Vec<String> {
        if !pattern.contains(['*', '?']) {
            return self.find_single_file(pattern, source, type_mask);
        }
        let prefix = source_prefix(source).to_owned();
        let mut list: Vec<String> = Vec::new();
        if self.settings.invalidate_older_files {
            for name in host_path::find_files(&self.working_directory, pattern) {
                list.insert(0, format!("{prefix}{name}"));
            }
        }
        if self.archives.is_empty() {
            return list;
        }
        let path = pattern.strip_prefix('\\').unwrap_or(pattern);
        let path = path
            .strip_prefix("Data\\")
            .or_else(|| path.strip_prefix("data\\"))
            .unwrap_or(path);
        let mask = if type_mask == 0xFFFF {
            self.types.lookup(last_characters(path, 3).unwrap_or(path))
        } else {
            type_mask
        };
        let hashes = bsa_path_hashes(path);
        // 0042ED55..0042ED6C the entries the list holds before the archives.
        let existing = list.len();
        let mut known: Option<Vec<u64>> = None;
        let settings = self.settings.clone();
        for archive in &mut self.archives {
            if u32::from(archive.types()) & mask == 0 {
                continue;
            }
            let Some(folder) = archive.find_folder(hashes.folder, None, &settings) else {
                continue;
            };
            if known.is_none() && existing != 0 {
                let mut hashed: Vec<u64> = list[..existing]
                    .iter()
                    .map(|entry| bsa_hash_path(entry, BsaHashMode::File))
                    .collect();
                hashed.sort_unstable();
                known = Some(hashed);
            }
            let records = archive.folders_178[folder as usize].files.clone();
            for (file, record) in records.iter().enumerate() {
                // 0042DC6F..0042DC98 (+0x194 bit 3) == offset bit 31 and a nonzero offset.
                if (u32::from(archive.runtime_flags_194 >> 3) ^ (record.offset >> 31)) & 1 != 0
                    || record.offset & 0x7FFF_FFFF == 0
                {
                    continue;
                }
                let compare = bsa_compare_hash(hashes.file, record.hash);
                if compare < 0 {
                    break;
                }
                if compare != 0
                    || known
                        .as_ref()
                        .is_some_and(|hashes| sorted_hashes_contain(hashes, record.hash))
                {
                    continue;
                }
                let name = archive
                    .read_file_name(folder, file as u32, &settings)
                    .expect("0042DD52: strlen of a null 0042CC00 name faults");
                list.insert(0, format!("{prefix}{name}"));
            }
        }
        list
    }

    /// IDA: 0042C970 ArchiveManager_ApplyArchiveCacheSettings (WinMain 0040EB72). Completion: 100%.
    pub fn apply_cache_settings(&mut self) {
        let (folders, files, offsets) = (
            self.settings.retain_directory_strings,
            self.settings.retain_filename_strings,
            self.settings.retain_filename_offsets,
        );
        let rule = |value: i32, flags: u32, bit: u32| {
            if value == 1 {
                flags & bit != 0
            } else {
                value != 0
            }
        };
        for archive in &mut self.archives {
            let flags = archive.archive_flags();
            if !rule(folders, flags, 0x08) {
                archive.release_folder_names();
            }
            if !rule(files, flags, 0x10) {
                archive.release_file_names(rule(offsets, flags, 0x20));
            }
        }
    }
}
