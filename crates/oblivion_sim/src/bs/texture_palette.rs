//! BSTexturePalette and the 007023E0 stream services backed by it.

use std::path::Path;

use bevy_ecs::prelude::*;
use oblivion_core::crt::{c_prefix, c_tolower, stricmp, strnicmp};
use oblivion_core::hash::ni_string_hash;

use super::archive::{ArchiveManager, BsaPathHashes, BsaRecordKey, bsa_path_hashes};
use super::file_finder::{BsFileCallbacks, FileFinder, bs_file_exists};
use crate::gamebryo::search_path::SearchPathCreatorSlot;
use crate::gamebryo::texture::SourceTextureLoadServices;

pub const PACKED_BUCKETS: u32 = 1009; // 0040E697 push 3F1h
pub const LOOSE_BUCKETS: u32 = 523; // 004A258A

/// IDA: N/A the archive layer takes `&str`; stream paths are bytes. Each byte becomes one char, as
/// `read_stream_object_table` does.
/// TODO: [ ] A non-ASCII byte reaches the BSA hash and the file open as two UTF-8 bytes. No shipped path has one.
fn path_text(path: &[u8]) -> String {
    path.iter().map(|&byte| char::from(byte)).collect()
}

// IDA: N/A a helper, with no address of its own.
fn path_bytes(path: &str) -> Vec<u8> {
    path.chars().map(|c| c as u8).collect()
}

// IDA: 0047D8F0 texture-path normalization: skip three characters when path[1] == ':', then one leading '\\', then a
// case-insensitive "data\\" prefix; copy (strcat into a 256-byte buffer) and _strlwr (C locale).
// Completion: 100%; inputs that overrun the 256-byte buffer are not bounded.
#[must_use]
pub fn normalize_texture_path(raw: &[u8]) -> Vec<u8> {
    let path = c_prefix(raw);
    let mut start = 0;
    if path.len() > 1 && path[1] == b':' {
        start = 3;
    }
    start = start.min(path.len());
    if start < path.len() && path[start] == b'\\' {
        start += 1;
    }
    if strnicmp(&path[start..], b"data\\", 5) == 0 {
        start += 5;
    }
    path[start.min(path.len())..]
        .iter()
        .map(|&byte| c_tolower(i32::from(byte)) as u8)
        .collect()
}

// IDA: 0042DB10(type, folderHash, fileHash, path): the first archive of the type (00B338E8[type]) or, only when that
// slot is null, the secondary one (00B3390C[type]); 0042D240 then runs Archive_FindFolder and
// Archive_FindFileInFolder(ignoreLazy = 1) with the path and returns the file record pointer. Host substitution: the
// record's identity in place of its pointer.
// Completion: 100%.
pub fn archive_file_entry(
    archives: Option<&mut ArchiveManager>,
    archive_type: u32,
    hashes: BsaPathHashes,
    path: &str,
) -> Option<BsaRecordKey> {
    archives?.file_record_by_type(archive_type, hashes, path)
}

/// IDA: BSTexturePalette (vtable A3FD80, 0x10 bytes: NiRefObject +0/+4, +0x08 NiTPointerMap<const BSFileEntry*,
/// NiPointer<NiTexture>> with 1009 buckets (vtable A3FD00, key % buckets), +0x0C NiTStringPointerMap<NiPointer<NiTexture>>
/// with 523 buckets (vtable A3FD60, h = 33 * h + signed byte, _stricmp equality, copied keys)). Each bucket is an explicit
/// list, head first. Completion: 90% scoped.
///
/// Host substitutions: the 00B353F8/00B353FC recursive critical section is exclusive access to the resource; the packed
/// map's key is the record identity and its bucket is `packed_bucket`, where the original's depends on a heap address (the
/// C++ port's too); the NiRefObject live-object counter (InterlockedIncrement at 004A252C) is not modelled.
/// TODO: [ ] Port the later-phase purge consumers (TES_PurgeUnusedResources).
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct BsTexturePalette {
    packed_08: Vec<Vec<(BsaRecordKey, Option<Entity>)>>,
    loose_0c: Vec<Vec<(Vec<u8>, Option<Entity>)>>,
    packed_count: usize,
    loose_count: usize,
}

impl Default for BsTexturePalette {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self::new(PACKED_BUCKETS)
    }
}

/// IDA: N/A see `BsTexturePalette`: a deterministic host stand-in for pointer % buckets. Only a future purge pass can
/// observe this order.
fn packed_bucket(key: BsaRecordKey, buckets: usize) -> usize {
    let mixed = (key.archive as u32) << 24 ^ key.folder << 12 ^ key.file;
    mixed as usize % buckets
}

impl BsTexturePalette {
    // IDA: 004A24F0 BSTexturePalette_ctor(1009) and 004A2100 map ctor.
    // Completion: 100%.
    #[must_use]
    pub fn new(packed_buckets: u32) -> Self {
        Self {
            packed_08: vec![Vec::new(); packed_buckets as usize],
            loose_0c: vec![Vec::new(); LOOSE_BUCKETS as usize],
            packed_count: 0,
            loose_count: 0,
        }
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn packed_count(&self) -> usize {
        self.packed_count
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn loose_count(&self) -> usize {
        self.loose_count
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn packed_bucket_count(&self) -> u32 {
        self.packed_08.len() as u32
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn loose_bucket_count(&self) -> u32 {
        self.loose_0c.len() as u32
    }

    // IDA: 007DAED0 bucket of a path key.
    // Completion: 100%.
    #[must_use]
    pub fn loose_bucket(&self, key: &[u8]) -> u32 {
        ni_string_hash(c_prefix(key)) % self.loose_bucket_count()
    }

    /// IDA: N/A inspection: keys of one path bucket, head first.
    #[must_use]
    pub fn loose_bucket_keys(&self, bucket: u32) -> Vec<Vec<u8>> {
        self.loose_0c
            .get(bucket as usize)
            .map(|chain| chain.iter().map(|entry| entry.0.clone()).collect())
            .unwrap_or_default()
    }

    // IDA: vt+4 004A1D80 Get(path): normalize, hash and 0042DB10(1, ...) outside the lock; inside it, the record map
    // (record found) or the path map (none) lookup. The original drops its temporary reference and returns a raw
    // pointer.
    // Completion: 100%.
    pub fn find(&self, path: &[u8], archives: Option<&mut ArchiveManager>) -> Option<Entity> {
        let normalized = normalize_texture_path(path);
        let text = path_text(&normalized);
        let hashes = bsa_path_hashes(&text);
        if let Some(record) = archive_file_entry(archives, 1, hashes, &text) {
            if self.packed_08.is_empty() {
                return None;
            }
            let chain = &self.packed_08[packed_bucket(record, self.packed_08.len())];
            return chain
                .iter()
                .find(|entry| entry.0 == record)
                .and_then(|entry| entry.1);
        }
        self.loose_0c[self.loose_bucket(&normalized) as usize]
            .iter()
            .find(|entry| stricmp(&entry.0, &normalized) == 0)
            .and_then(|entry| entry.1)
    }

    // IDA: vt+8 004A2000 Add(path, texture): inside the lock normalize, hash and 0042DB10(1, ...), then 006AA3B0 (record
    // map) or 004A1B10 (path map) SetAt: an existing key replaces its value, a new node is pushed at the head of its
    // bucket and the count incremented. A null texture is stored too (007B82CD adds a failed creation).
    // Completion: 100%.
    pub fn add(
        &mut self,
        path: &[u8],
        texture: Option<Entity>,
        archives: Option<&mut ArchiveManager>,
    ) {
        let normalized = normalize_texture_path(path);
        let text = path_text(&normalized);
        let hashes = bsa_path_hashes(&text);
        if let Some(record) = archive_file_entry(archives, 1, hashes, &text) {
            if self.packed_08.is_empty() {
                return;
            }
            let bucket = packed_bucket(record, self.packed_08.len());
            let chain = &mut self.packed_08[bucket];
            if let Some(entry) = chain.iter_mut().find(|entry| entry.0 == record) {
                entry.1 = texture;
                return;
            }
            chain.insert(0, (record, texture));
            self.packed_count += 1;
            return;
        }
        let bucket = self.loose_bucket(&normalized) as usize;
        let chain = &mut self.loose_0c[bucket];
        if let Some(entry) = chain
            .iter_mut()
            .find(|entry| stricmp(&entry.0, &normalized) == 0)
        {
            entry.1 = texture;
            return;
        }
        chain.insert(0, (normalized, texture));
        self.loose_count += 1;
    }
}

/// IDA: 007023E0 stream services backed by the converted originals: the 00B40334 creator slot's search path (stream +1E8,
/// BSSearchPath after 0040E63D; the unported base NiSearchPath walk yields no candidates), the 00B27E50 exists callback
/// with mode 0 and the palette's vt+4/vt+8 (stream +254 = alias 00B3FAC8). A null palette (allocation failure) returns no
/// texture and ignores registration where the original would dereference null. Built by
/// `make_source_texture_services`, with the world's resources split out by the caller.
/// Completion: 100% of the routing.
pub struct ConvertedTextureServices<'a> {
    pub palette: Option<&'a mut BsTexturePalette>,
    pub archives: Option<&'a mut ArchiveManager>,
    pub finder: Option<&'a FileFinder>,
    pub callbacks: &'a BsFileCallbacks,
    pub default_root: &'a Path,
    pub search_path_slot: SearchPathCreatorSlot,
}

/// IDA: N/A `make_source_texture_services` in the C++ port: the plain constructor of `ConvertedTextureServices`.
#[must_use]
pub const fn make_source_texture_services<'a>(
    palette: Option<&'a mut BsTexturePalette>,
    archives: Option<&'a mut ArchiveManager>,
    finder: Option<&'a FileFinder>,
    callbacks: &'a BsFileCallbacks,
    default_root: &'a Path,
    search_path_slot: SearchPathCreatorSlot,
) -> ConvertedTextureServices<'a> {
    ConvertedTextureServices {
        palette,
        archives,
        finder,
        callbacks,
        default_root,
        search_path_slot,
    }
}

impl SourceTextureLoadServices for ConvertedTextureServices<'_> {
    /// IDA: N/A `ConvertedTextureServices`'s side of `SourceTextureLoadServices`.
    fn search_candidates(&mut self, file_path: &[u8]) -> Vec<Vec<u8>> {
        let mut search = self.search_path_slot.create();
        search.set_file_path(Some(&path_text(file_path)));
        search.reset();
        let mut candidates = Vec::new();
        while let Some(next) = search.next_search_path() {
            candidates.push(path_bytes(&next));
        }
        candidates
    }

    /// IDA: N/A `ConvertedTextureServices`'s side of `SourceTextureLoadServices`.
    fn find_texture(&mut self, path: &[u8]) -> Option<Entity> {
        self.palette
            .as_deref()?
            .find(path, self.archives.as_deref_mut())
    }

    /// IDA: N/A `ConvertedTextureServices`'s side of `SourceTextureLoadServices`.
    fn register_texture(&mut self, path: &[u8], texture: Entity) {
        if let Some(palette) = self.palette.as_deref_mut() {
            palette.add(path, Some(texture), self.archives.as_deref_mut());
        }
    }

    /// IDA: N/A `ConvertedTextureServices`'s side of `SourceTextureLoadServices`.
    fn file_exists(&mut self, path: &[u8]) -> bool {
        bs_file_exists(
            self.callbacks,
            self.default_root,
            self.finder,
            self.archives.as_deref_mut(),
            &path_text(path),
            0,
        )
    }
}
