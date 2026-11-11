//! TESFile (ModEntry::Data, 0x41C): the record and chunk reader of Phases 09 and 12, with compressed records (D2).
//!
//! Phase 09's `TesFileData` is the field-exact TESFile the plugin list holds. M0.6's bounded `TesFile` is folded into it. Chunk functions take the file; record functions take the
//! `TesFileStore` and the id, since 00450080 reads a resolved master's +0x400. `RecordReader` is the handle a LoadForm uses.

use flate2::{Decompress, FlushDecompress, Status};
use oblivion_core::crt::stricmp;
use oblivion_core::error::GameError;
use oblivion_core::host_path;

use crate::bs::file::{BsFile, GameFile, NiFileMode, SEEK_SET};
use crate::bs::string::BsStringT;
use crate::tes::data_handler::DataHandler;
use crate::tes::form::record_fourcc;
use crate::tes::form_arms::FormLoadCx;
use crate::tes::tes::TesGlobals;
use bevy_ecs::prelude::*;

// IDA: N/A a helper, with no address of its own.
fn dword(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("a dword"))
}

/// IDA: 0046ABD0 the reserved form-ID test (1..0x7FF). Completion: 100%.
#[must_use]
pub const fn reserved_form_id(id: u32) -> bool {
    id != 0 && id <= 0x7FF
}

/// IDA: 00B05E14 / 00B05E20 / 00B06138 the TES4, GRUP and TOFT fourccs.
pub const TES4_FOURCC: u32 = 0x3453_4554;
pub const GRUP_FOURCC: u32 = 0x5055_5247;
pub const TOFT_FOURCC: u32 = 0x5446_4F54;
// IDA: the chunk fourccs 00451A60 dispatches on.
const CNAM_FOURCC: u32 = 0x4D41_4E43;
const SNAM_FOURCC: u32 = 0x4D41_4E53;
const DATA_FOURCC: u32 = 0x4154_4144;
const HEDR_FOURCC: u32 = 0x5244_4548;
const MAST_FOURCC: u32 = 0x5453_414D;
const XXXX_FOURCC: u32 = 0x5858_5858;
// IDA: record flag 0x40000 (compressed) checked by the chunk readers.
const COMPRESSED_RECORD: u32 = 0x0004_0000;
// IDA: the {1} component-size array the readers pass to the read proc.
const BYTE_COMPONENTS: [u32; 1] = [1];

/// IDA: 00B05E00 the 69-entry form-type table {u8 type, char* name, u32 fourcc} (12 bytes each; NONE .. TOFT). Completion: 100%.
pub const FORM_TYPE_TABLE: [(u8, &str); 69] = [
    (0x00, "NONE"),
    (0x01, "TES4"),
    (0x02, "GRUP"),
    (0x03, "GMST"),
    (0x04, "GLOB"),
    (0x05, "CLAS"),
    (0x06, "FACT"),
    (0x07, "HAIR"),
    (0x08, "EYES"),
    (0x09, "RACE"),
    (0x0A, "SOUN"),
    (0x0B, "SKIL"),
    (0x0C, "MGEF"),
    (0x0D, "SCPT"),
    (0x0E, "LTEX"),
    (0x0F, "ENCH"),
    (0x10, "SPEL"),
    (0x11, "BSGN"),
    (0x12, "ACTI"),
    (0x13, "APPA"),
    (0x14, "ARMO"),
    (0x15, "BOOK"),
    (0x16, "CLOT"),
    (0x17, "CONT"),
    (0x18, "DOOR"),
    (0x19, "INGR"),
    (0x1A, "LIGH"),
    (0x1B, "MISC"),
    (0x1C, "STAT"),
    (0x1D, "GRAS"),
    (0x1E, "TREE"),
    (0x1F, "FLOR"),
    (0x20, "FURN"),
    (0x21, "WEAP"),
    (0x22, "AMMO"),
    (0x23, "NPC_"),
    (0x24, "CREA"),
    (0x25, "LVLC"),
    (0x26, "SLGM"),
    (0x27, "KEYM"),
    (0x28, "ALCH"),
    (0x29, "SBSP"),
    (0x2A, "SGST"),
    (0x2B, "LVLI"),
    (0x2C, "SNDG"),
    (0x2D, "WTHR"),
    (0x2E, "CLMT"),
    (0x2F, "REGN"),
    (0x30, "CELL"),
    (0x31, "REFR"),
    (0x32, "ACHR"),
    (0x33, "ACRE"),
    (0x34, "PGRD"),
    (0x35, "WRLD"),
    (0x36, "LAND"),
    (0x37, "TLOD"),
    (0x38, "ROAD"),
    (0x39, "DIAL"),
    (0x3A, "INFO"),
    (0x3B, "QUST"),
    (0x3C, "IDLE"),
    (0x3D, "PACK"),
    (0x3E, "CSTY"),
    (0x3F, "LSCR"),
    (0x40, "LVSP"),
    (0x41, "ANIO"),
    (0x42, "WATR"),
    (0x43, "EFSH"),
    (0x44, "TOFT"),
];

/// IDA: 00B33C1C / 00B33C20, the 0046AF70 cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FormTypeLookupCache {
    pub last_type_b33c1c: u32,
    pub last_fourcc_b33c20: u32,
}

// IDA: 0046AF70 FormType_FromRecordFourCC(fourcc): the cached last result (00B33C20 fourcc, 00B33C1C type) when the fourcc matches; otherwise the table index of
// the first match (cached), or 0 without caching. Completion: 100%.
pub fn form_type_from_record_fourcc(fourcc: u32, cache: &mut FormTypeLookupCache) -> u32 {
    if fourcc == cache.last_fourcc_b33c20 {
        return cache.last_type_b33c1c;
    }
    for (index, (_, name)) in FORM_TYPE_TABLE.iter().enumerate() {
        if record_fourcc(name) == fourcc {
            cache.last_type_b33c1c = index as u32;
            cache.last_fourcc_b33c20 = fourcc;
            return index as u32;
        }
    }
    0
}

/// IDA: the 0x14-byte record header at +0x23C the readers fill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesRecordHeader {
    pub type_23c: u32,
    pub size_240: u32,
    pub flags_244: u32,
    pub form_id_248: u32,
    pub tracking_24c: u32,
}

/// IDA: the WIN32_FIND_DATAA at +0x290 (0x140 bytes), with FILETIMEs as raw u64. `file_name` keeps the name on disk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindData {
    pub attributes: u32,
    pub creation_time: u64,
    pub last_access_time: u64,
    pub last_write_time: u64,
    pub size_high: u32,
    pub size_low: u32,
    pub file_name: String,
}

// IDA: N/A a Unix time as a FILETIME: 100 ns units since 1601. Host substitution.
fn file_time(time: std::io::Result<std::time::SystemTime>) -> u64 {
    time.ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| {
            (since.as_nanos() / 100) as u64 + 116_444_736_000_000_000
        })
}

// IDA: N/A FindFirstFileA / FindNextFileA substitute: `host_path::find_files` gives the names in NTFS order, and each becomes the A record. dwFileAttributes has
// no Linux source: a regular file is FILE_ATTRIBUTE_ARCHIVE (0x20), the value every recorded plugin has, and a directory 0x10 (host substitution).
fn find_all(working: &std::path::Path, pattern: &str) -> Vec<FindData> {
    let directory = pattern
        .rsplit_once('\\')
        .map_or(String::new(), |(directory, _)| format!("{directory}\\"));
    host_path::find_files(working, pattern)
        .into_iter()
        .filter_map(|name| {
            let path = host_path::resolve(working, &format!("{directory}{name}"));
            let metadata = std::fs::metadata(path).ok()?;
            let size = metadata.len();
            Some(FindData {
                attributes: if metadata.is_dir() { 0x10 } else { 0x20 },
                creation_time: file_time(metadata.created()),
                last_access_time: file_time(metadata.accessed()),
                last_write_time: file_time(metadata.modified()),
                size_high: (size >> 32) as u32,
                size_low: size as u32,
                file_name: name,
            })
        })
        .collect()
}

/// IDA: an index into `TesFileStore`, standing for a TESFile* (host substitution).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TesFileId(pub usize);

/// IDA: TESFile (ModEntry::Data, 0x41C; xOBSE GameData.h). The constructor 00451DA0 stores the fields named below; unnamed bytes (+0x14/+0x18, +0x268..+0x278) stay
/// uninitialized FormHeap bytes (not modelled).
#[derive(Default)]
pub struct TesFileData {
    pub error_state_00: u32,
    pub ghost_parent_04: u32,
    pub ghost_children_08: u32,
    /// unkFile00C (write path, unused here).
    pub file_0c: u32,
    pub bs_file_10: Option<BsFile>,
    /// +0x1C char[0x104].
    pub name_1c: String,
    /// +0x120 char[0x104].
    pub directory_120: String,
    pub field_224: u32,
    /// *(00B055CC), 0x2800.
    pub buffer_size_228: u32,
    pub fields_22c: [u32; 4],
    pub record_23c: TesRecordHeader,
    pub chunk_type_250: u32,
    pub chunk_size_254: u32,
    pub file_size_258: u32,
    pub record_offset_25c: u32,
    pub chunk_offset_260: u32,
    pub fetched_264: u32,
    pub field_27c: u32,
    pub field_280: u32,
    pub open_groups_284: [u32; 2],
    pub header_read_28c: u8,
    pub find_data_290: FindData,
    /// HEDR: version, form count, next form ID.
    pub header_3d0: [u32; 3],
    pub flags_3dc: u32,
    /// +0x3E0 BSSimpleList<char*>: the MAST strings, in list order.
    pub masters_3e0: Vec<Vec<u8>>,
    /// +0x3E8 BSSimpleList: the DATA blocks (8 bytes), in list order.
    pub master_sizes_3e8: Vec<[u8; 8]>,
    pub master_count_3f0: u32,
    /// 0044FC70's resolved masters.
    pub master_files_3f4: Vec<Option<TesFileId>>,
    /// +0x3F4 non-null.
    pub master_files_allocated: bool,
    pub field_3f8: u32,
    pub field_3fc: u32,
    pub mod_index_400: u8,
    pub author_404: BsStringT,
    pub description_40c: BsStringT,
    /// +0x414 presence: Phase 12 fills it.
    pub decompressed_414: Option<Vec<u8>>,
    pub decompressed_length_418: u32,
}

/// IDA: N/A the owner of the TESFile objects the list references (the original's FormHeap blocks). A slot stays `None` after 0044A5F1 destroys its file, so ids
/// stay stable.
#[derive(Default)]
pub struct TesFileStore {
    pub files: Vec<Option<TesFileData>>,
}

impl TesFileStore {
    /// IDA: N/A a file by id.
    #[must_use]
    pub fn get(&self, id: TesFileId) -> Option<&TesFileData> {
        self.files.get(id.0).and_then(Option::as_ref)
    }

    /// IDA: N/A a live file by id; a destroyed one is a host stop.
    pub fn get_mut(&mut self, id: TesFileId) -> Result<&mut TesFileData, GameError> {
        self.files
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .ok_or(GameError::unclassified(
                0,
                "a TESFile list entry names a destroyed file",
            ))
    }

    /// IDA: N/A adds a file, as a FormHeap block the list then names.
    pub fn push(&mut self, file: TesFileData) -> TesFileId {
        self.files.push(Some(file));
        TesFileId(self.files.len() - 1)
    }

    /// IDA: N/A the live files in construction order.
    pub fn iter(&self) -> impl Iterator<Item = &TesFileData> {
        self.files.iter().flatten()
    }
}

/// IDA: N/A the services the file list functions use.
pub struct TesFileServices<'a> {
    /// The directory the relative "Data\\" path resolves against (the original's process directory).
    pub working_directory: &'a std::path::Path,
    /// 004A7A60 DebugMessage_Warning(text, g_disableWarningMessages at the call) (D4: formatted by the host, the flag is its state).
    pub warning: &'a mut dyn FnMut(String, bool),
    /// 00B06B18 g_disableWarningMessages (bDisableWarning:MESSAGES).
    pub disable_warning_messages_b06b18: u8,
    /// 00B33A98 g_dataHandler present and its +0xCD4 byte (004507D0's early return).
    pub close_suppressed: bool,
    pub form_type_cache: &'a mut FormTypeLookupCache,
}

impl TesFileServices<'_> {
    /// IDA: 004A7A60 DebugMessage_Warning (D4: the host log, with the flag state). Completion: 100% of the text.
    pub fn warn(&mut self, text: String) {
        let disabled = self.disable_warning_messages_b06b18 != 0;
        (self.warning)(text, disabled);
    }
}

// IDA: the +0x23C header cleared. Completion: 100%.
fn clear_record(file: &mut TesFileData) {
    file.record_23c = TesRecordHeader::default();
}

// IDA: the BSFile read proc (+0x04) with the {1} component array. Completion: 100%.
fn read_file(file: &mut TesFileData, buffer: &mut [u8]) -> Result<usize, GameError> {
    let stream = file.bs_file_10.as_mut().ok_or(GameError::unclassified(
        0,
        "a TESFile read with a null +0x10 BSFile",
    ))?;
    Ok(stream.read_components(buffer, Some(&BYTE_COMPONENTS[..])))
}

// IDA: the BSFile vt+0x10 seek, from the start. Completion: 100%.
fn seek_file(file: &mut TesFileData, offset: u32) -> Result<(), GameError> {
    let stream = file.bs_file_10.as_mut().ok_or(GameError::unclassified(
        0,
        "a TESFile seek with a null +0x10 BSFile",
    ))?;
    stream.seek(offset as i32, SEEK_SET);
    Ok(())
}

// IDA: 0044FD60 TESFile_GetMaster(index): 0 past the count, this for index 0, the +0x3F4 slot index - 1 otherwise (0 without the array). Completion: 100%.
#[must_use]
pub fn master_at(file: &TesFileData, this: TesFileId, index: u32) -> Option<TesFileId> {
    if index > file.master_count_3f0 {
        return None;
    }
    if index == 0 {
        return Some(this);
    }
    if !file.master_files_allocated {
        return None;
    }
    file.master_files_3f4
        .get(index as usize - 1)
        .copied()
        .flatten()
}

// IDA: 00450080 TESFileData_ReadNextRecordHeader: 0 with a null +0x10; a read of nothing clears the header and returns 0; a GRUP or TOFT keeps its words. Otherwise, with +0x3F4 set, 0044FD60(top byte + 1) names
// a master: its +0x400 replaces the top byte. With no master, an ID below the 0xFF top byte whose low 24 bits are reserved (0046ABD0) clears the top byte (00450154), and any other takes this file's +0x400. A short
// read that isn't empty goes on with what it read. Completion: 100%.
fn read_next_record_header(files: &mut TesFileStore, id: TesFileId) -> Result<bool, GameError> {
    let file = files.get_mut(id)?;
    if file.bs_file_10.is_none() {
        return Ok(false);
    }
    // 0045009B reads straight into +0x23C, so a short read keeps the old tail (the C++ zeroes it).
    let old = &file.record_23c;
    let mut header = [0_u8; 0x14];
    for (at, word) in [
        old.type_23c,
        old.size_240,
        old.flags_244,
        old.form_id_248,
        old.tracking_24c,
    ]
    .into_iter()
    .enumerate()
    {
        header[at * 4..at * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    if read_file(file, &mut header)? == 0 {
        clear_record(file);
        return Ok(false);
    }
    let word = |at: usize| dword(&header, at);
    file.record_23c = TesRecordHeader {
        type_23c: word(0),
        size_240: word(4),
        flags_244: word(8),
        form_id_248: word(12),
        tracking_24c: word(16),
    };
    let kind = file.record_23c.type_23c;
    if kind == GRUP_FOURCC || kind == TOFT_FOURCC {
        return Ok(true);
    }
    let raw = file.record_23c.form_id_248;
    let own_index = file.mod_index_400;
    // 004500E6..004500FB the master slot, read before the store below. 0044FD60 can't take its index-0 arm: the index is at least 1.
    let master = master_at(file, id, (raw >> 24) + 1);
    let master_index = match master {
        Some(master) if master == id => Some(own_index),
        Some(master) => Some(files.get_mut(master)?.mod_index_400),
        None => None,
    };
    let low = raw & 0x00FF_FFFF;
    let mapped = if let Some(index) = master_index {
        (u32::from(index) << 24) | low
    } else if raw & 0xFF00_0000 != 0xFF00_0000 && reserved_form_id(low) {
        low
    } else {
        (u32::from(own_index) << 24) | low
    };
    files.get_mut(id)?.record_23c.form_id_248 = mapped;
    Ok(true)
}

// IDA: N/A a non-group record with record flag 0x40000: the test 00450F33, 0044FEBD and 00450C72 repeat.
const fn compressed_record(file: &TesFileData) -> bool {
    file.record_23c.type_23c != GRUP_FOURCC && file.record_23c.flags_244 & COMPRESSED_RECORD != 0
}

/// IDA: 0044FB60 TESFile_FreeDecompressedBuffer: frees a nonzero +0x414 and clears +0x414 and +0x418. Completion: 100%.
pub fn free_decompressed_buffer(file: &mut TesFileData) {
    if file.decompressed_414.is_some() {
        file.decompressed_414 = None;
        file.decompressed_length_418 = 0;
    }
}

// IDA: 00450550 TESFileData_DecompressCurrentRecord. A record with data, not a group, with flag 0x40000 and a known form type (0046AF70's al nonzero): FormHeap(size + 1); the BSFile reads the record (42C8E0, from where the file stands:
// right after the header); a short read warns and frees the block. Byte [size] = 0; the dword at [0] is the inflated length; +0x414 = FormHeap(length), +0x418 = length; inflateInit_("1.2.1", 0x38): a failure runs inflateEnd, warns, 0044FB60 and
// returns without freeing the input (the leak isn't modelled). One inflate(Z_NO_FLUSH) of size - 4 bytes from [4] into the buffer: 1 is clean; -2, 2, -3, -4 warn "Error inflating"; any other result warns "did not terminate"; both then run 0044FB60 and free the input.
// Host substitution (D2): flate2 for the linked zlib 1.2.1, with the same output for a valid stream. Its Err covers -2, 2, -3 and -4; `Status::Ok` and `BufError` are the "any other result" arm. inflateInit_ can't fail here, so that arm isn't reachable. The input block is a local
// `Vec`, padded so a record shorter than 4 bytes reads zeros where the original reads its heap block. Completion: 100% of the control flow and texts.
fn decompress_current_record(
    file: &mut TesFileData,
    services: &mut TesFileServices<'_>,
) -> Result<(), GameError> {
    let size = file.record_23c.size_240;
    if size == 0 || !compressed_record(file) {
        return Ok(());
    }
    // 00450586..00450590 `test al, al` on the form type.
    if form_type_from_record_fourcc(file.record_23c.type_23c, services.form_type_cache) as u8 == 0 {
        return Ok(());
    }
    let mut input = vec![0_u8; (size as usize + 1).max(4)];
    if read_file(file, &mut input[..size as usize])? != size as usize {
        services.warn("TESFile: Failed to read in buffer data for compressed form.".to_owned());
        return Ok(());
    }
    let length = dword(&input, 0);
    input[size as usize] = 0;
    // 004505F6..0045060F +0x414, +0x418.
    file.decompressed_414 = Some(vec![0_u8; length as usize]);
    file.decompressed_length_418 = length;
    let mut stream = Decompress::new(true);
    let available = (size as usize).saturating_sub(4);
    let status = stream.decompress(
        &input[4..4 + available],
        file.decompressed_414.as_mut().expect("just stored"),
        FlushDecompress::None,
    );
    match status {
        Ok(Status::StreamEnd) => {}
        Err(_) => {
            services.warn("TESFile: Error inflating ZLib stream.".to_owned());
            free_decompressed_buffer(file);
        }
        Ok(Status::Ok | Status::BufError) => {
            services.warn("TESFile: Error: ZLib stream did not terminate.".to_owned());
            free_decompressed_buffer(file);
        }
    }
    Ok(())
}

// IDA: 00450F10 TESFileData_ReadNextChunkHeader. A compressed record takes its 6 bytes from the +0x414 buffer at +0x260, reading +0x260 before 00450550 and decompressing when the buffer is absent; no buffer or an offset at or past +0x418 clears the chunk and returns 0.
// The original reads the 6 bytes with no further bound; the host reads zeros past the end. An uncompressed record reads the BSFile, and a read of nothing clears the chunk. XXXX reads its dword (a 00450C20 with size 0) and advances, the size becoming that value. Completion: 100%.
fn read_next_chunk_header(
    file: &mut TesFileData,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
    let mut header = [0_u8; 6];
    if compressed_record(file) {
        let offset = file.chunk_offset_260;
        if file.decompressed_414.is_none() {
            decompress_current_record(file, services)?;
        }
        let Some(buffer) = file.decompressed_414.as_ref() else {
            file.chunk_size_254 = 0;
            file.chunk_type_250 = 0;
            return Ok(false);
        };
        if offset >= file.decompressed_length_418 {
            file.chunk_size_254 = 0;
            file.chunk_type_250 = 0;
            return Ok(false);
        }
        for (index, byte) in header.iter_mut().enumerate() {
            *byte = buffer.get(offset as usize + index).copied().unwrap_or(0);
        }
    } else if read_file(file, &mut header)? == 0 {
        file.chunk_size_254 = 0;
        file.chunk_type_250 = 0;
        return Ok(false);
    }
    let kind = dword(&header, 0);
    file.chunk_type_250 = kind;
    file.chunk_size_254 = u32::from(u16::from_le_bytes([header[4], header[5]]));
    if kind == XXXX_FOURCC {
        let mut extended = vec![0_u8; (file.chunk_size_254 as usize).max(4)];
        read_current_chunk_data(file, &mut extended, 0, services)?;
        advance_chunk(file, services)?;
        file.chunk_size_254 = dword(&extended, 0);
    }
    Ok(true)
}

/// IDA: 0044FEA0 TESFileData_AdvanceChunk: the limit is +0x418 for a compressed record, else the record size; +0x260 += size + 6; at or past the limit clears the chunk and returns false; otherwise an uncompressed record seeks to +0x25C + +0x260 + 0x14, +0x264 and the chunk clear, and
/// 00450F10 reads the next header, whose result is ignored. Completion: 100%.
pub fn advance_chunk(
    file: &mut TesFileData,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
    let compressed = compressed_record(file);
    let limit = if compressed {
        file.decompressed_length_418
    } else {
        file.record_23c.size_240
    };
    file.chunk_offset_260 = file
        .chunk_offset_260
        .wrapping_add(file.chunk_size_254)
        .wrapping_add(6);
    if file.chunk_offset_260 >= limit {
        file.chunk_type_250 = 0;
        file.chunk_size_254 = 0;
        return Ok(false);
    }
    if !compressed {
        seek_file(
            file,
            file.record_offset_25c
                .wrapping_add(file.chunk_offset_260)
                .wrapping_add(0x14),
        )?;
    }
    file.fetched_264 = 0;
    file.chunk_type_250 = 0;
    file.chunk_size_254 = 0;
    read_next_chunk_header(file, services)?;
    Ok(true)
}

// IDA: N/A a fourcc as the `%s` of a local char[5]: it stops at a NUL. Bytes lift to Latin-1 in a host warning.
fn fourcc_text(value: u32) -> String {
    latin1(c_bytes(&value.to_le_bytes()))
}

// IDA: N/A bytes as a host log string (Latin-1).
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

// IDA: N/A a compressed chunk's bytes, from 00450D76 and 00450ED9's memcpy from +0x414 + +0x260 + 6 (the original has no bound; the host leaves zeros past the end). A null buffer faults there: a host stop.
fn copy_from_decompressed(
    file: &TesFileData,
    destination: &mut [u8],
    site: &'static str,
) -> Result<(), GameError> {
    let buffer = file
        .decompressed_414
        .as_ref()
        .ok_or(GameError::from_legacy_message(site))?;
    let start = file.chunk_offset_260 as usize + 6;
    for (index, byte) in destination.iter_mut().enumerate() {
        *byte = buffer.get(start + index).copied().unwrap_or(0);
    }
    Ok(())
}

/// IDA: 00450C20 TESFileData_ReadCurrentChunkData(destination, size): true for an empty chunk. A re-read (+0x264 > 0) seeks back to +0x260 + +0x25C + 0x1A on an uncompressed record and clears +0x264. A nonzero size smaller than the chunk stores a NUL at [size - 1], reads size - 1 bytes
/// (+0x264 = what was read) and warns "Chunk size ... too big"; otherwise the whole chunk is read. Compressed records copy from the +0x414 buffer, decompressing first when it is absent. A short read warns and returns false. `destination` must hold the bytes read: `size` when
/// nonzero and smaller than the chunk, else the chunk size. Windows' error text in the short-read warnings is the host's. Completion: 100%.
pub fn read_current_chunk_data(
    file: &mut TesFileData,
    destination: &mut [u8],
    destination_size: u32,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
    if file.chunk_size_254 == 0 {
        return Ok(true);
    }
    let compressed = compressed_record(file);
    // 00450C54..00450C9B.
    if file.fetched_264 > 0 {
        if !compressed {
            seek_file(
                file,
                file.chunk_offset_260
                    .wrapping_add(file.record_offset_25c)
                    .wrapping_add(0x1A),
            )?;
        }
        file.fetched_264 = 0;
    }
    if destination_size != 0 && file.chunk_size_254 > destination_size {
        // 00450CBD the NUL; 00450CD8..00450D7E the read.
        let want = destination_size as usize - 1;
        destination[want] = 0;
        if compressed {
            if file.decompressed_414.is_none() {
                decompress_current_record(file, services)?;
            }
            copy_from_decompressed(
                file,
                &mut destination[..want],
                "00450D76: memcpy from a null decompressed buffer",
            )?;
            file.fetched_264 = want as u32;
        } else {
            let read = read_file(file, &mut destination[..want])?;
            file.fetched_264 = read as u32;
            if read != want {
                services.warn(format!(
                    "First ReadFile() in GetChunkData failed with error:\n{}",
                    std::io::Error::last_os_error()
                ));
                return Ok(false);
            }
        }
        // 00450D84..00450DFD the warning, whose last argument is the destination text.
        let text = format!(
            "Chunk size {} too big in chunk {}_ID in form {}_ID.\r\nMax size is {}, data truncated to \"{}\".\r\n",
            file.chunk_size_254 as i32,
            fourcc_text(file.chunk_type_250),
            fourcc_text(file.record_23c.type_23c),
            destination_size as i32,
            latin1(c_bytes(&destination[..=want])),
        );
        services.warn(text);
        return Ok(true);
    }
    let want = file.chunk_size_254 as usize;
    if compressed {
        // 00450EAF..00450EE7.
        if file.decompressed_414.is_none() {
            decompress_current_record(file, services)?;
        }
        copy_from_decompressed(
            file,
            &mut destination[..want],
            "00450ED9: memcpy from a null decompressed buffer",
        )?;
        file.fetched_264 = file.chunk_size_254;
        return Ok(true);
    }
    let read = read_file(file, &mut destination[..want])?;
    file.fetched_264 = read as u32;
    if read != want {
        services.warn(format!(
            "Second ReadFile() in GetChunkData failed with error:\n{}",
            std::io::Error::last_os_error()
        ));
        return Ok(false);
    }
    Ok(true)
}

/// IDA: 004510B0 TESFileData_GetCurrentChunkTypeFourCC: +0x250, reading the next header while it is 0 (0 when that returns 0). Completion: 100%.
pub fn current_chunk_type(
    file: &mut TesFileData,
    services: &mut TesFileServices<'_>,
) -> Result<u32, GameError> {
    if file.chunk_type_250 == 0 && !read_next_chunk_header(file, services)? {
        return Ok(0);
    }
    Ok(file.chunk_type_250)
}

/// IDA: 004511C0 TESFileData_GetCurrentRecordFormType. Completion: 100%.
pub fn current_record_form_type(
    files: &mut TesFileStore,
    id: TesFileId,
    services: &mut TesFileServices<'_>,
) -> Result<u32, GameError> {
    if files.get_mut(id)?.record_23c.type_23c == 0 {
        if !read_next_record_header(files, id)? {
            return Ok(0);
        }
        let file = files.get_mut(id)?;
        file.chunk_type_250 = 0;
        file.chunk_size_254 = 0;
        read_next_chunk_header(file, services)?;
    }
    Ok(form_type_from_record_fourcc(
        files.get_mut(id)?.record_23c.type_23c,
        services.form_type_cache,
    ))
}

/// IDA: 004512A0 TESFileData_Rewind(readHeader): seek 0, clear the offsets and the record header, then (readHeader) 004511C0. Completion: 100%.
/// # Errors
/// A read failure in the host.
pub fn rewind_tes_file(
    files: &mut TesFileStore,
    id: TesFileId,
    read_header: bool,
    services: &mut TesFileServices<'_>,
) -> Result<(), GameError> {
    let file = files.get_mut(id)?;
    if file.bs_file_10.is_some() {
        seek_file(file, 0)?;
    }
    file.record_offset_25c = 0;
    file.chunk_offset_260 = 0;
    file.fetched_264 = 0;
    clear_record(file);
    if read_header {
        current_record_form_type(files, id, services)?;
    }
    Ok(())
}

// IDA: 00451310 TESFileData_NextForm. Completion: 100%; the error text of the seek warning is the host's.
fn next_form(
    files: &mut TesFileStore,
    id: TesFileId,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
    let file = files.get_mut(id)?;
    let kind = file.record_23c.type_23c;
    file.record_offset_25c = if kind == GRUP_FOURCC || kind == TOFT_FOURCC {
        file.record_offset_25c.wrapping_add(0x14)
    } else {
        file.record_offset_25c
            .wrapping_add(file.record_23c.size_240)
            .wrapping_add(0x14)
    };
    if file.record_offset_25c >= file.file_size_258 {
        clear_record(file);
        return Ok(false);
    }
    seek_file(file, file.record_offset_25c)?;
    let position = file
        .bs_file_10
        .as_ref()
        .map_or(0, BsFile::effective_position);
    file.record_offset_25c = position;
    if position == 0xFFFF_FFFF {
        services.warn(format!(
            "SetFilePointer() in NextForm failed with error:\n{}",
            std::io::Error::last_os_error()
        ));
        return Ok(false);
    }
    file.chunk_offset_260 = 0;
    file.fetched_264 = 0;
    clear_record(file);
    current_record_form_type(files, id, services)?;
    let file = files.get_mut(id)?;
    if file.record_23c.type_23c == 0 {
        let text = format!(
            "Trying to load a bad form in TESFile::NextForm.\r\nFile = {}\r\nOffset = {}",
            file.name_1c, file.record_offset_25c as i32
        );
        services.warn(text);
        return Ok(false);
    }
    Ok(true)
}

/// IDA: 004518B0 TESFileData_SkipRecord(skipIgnored): frees the decompressed buffer, NextForm, and with skipIgnored repeats while the next record is not a group or TOFT and has flag 0x1000. Returns the last NextForm result. Completion: 100%.
pub fn skip_record(
    files: &mut TesFileStore,
    id: TesFileId,
    skip_ignored: bool,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
    free_decompressed_buffer(files.get_mut(id)?);
    let mut more = next_form(files, id, services)?;
    while more && skip_ignored {
        let file = files.get_mut(id)?;
        let kind = file.record_23c.type_23c;
        if kind == GRUP_FOURCC || kind == TOFT_FOURCC || file.record_23c.flags_244 & 0x1000 == 0 {
            break;
        }
        more = next_form(files, id, services)?;
    }
    Ok(more)
}

/// IDA: 00451460 TESFileData_SetOffset(offset): the decompressed buffer freed and +0x418 = 0; +0x25C = offset; at or past +0x258 the header clears and it answers false; the seek; +0x25C from the BSFile's position, -1 warns and answers false; +0x260, +0x264 and the header clear; 004511C0, whose answer is dropped; true. Completion: 100%.
/// # Errors
/// A read failure in the host.
pub fn set_tes_file_offset(
    files: &mut TesFileStore,
    id: TesFileId,
    offset: u32,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
    let file = files.get_mut(id)?;
    if file.decompressed_414.is_some() {
        file.decompressed_414 = None;
        file.decompressed_length_418 = 0;
    }
    file.record_offset_25c = offset;
    if offset >= file.file_size_258 {
        clear_record(file);
        return Ok(false);
    }
    seek_file(file, offset)?;
    let position = file
        .bs_file_10
        .as_ref()
        .map_or(0, BsFile::effective_position);
    file.record_offset_25c = position;
    if position == 0xFFFF_FFFF {
        services.warn(format!(
            "SetFilePointer() in SetOffset failed with error:\n{}",
            std::io::Error::last_os_error()
        ));
        return Ok(false);
    }
    file.chunk_offset_260 = 0;
    file.fetched_264 = 0;
    clear_record(file);
    current_record_form_type(files, id, services)?;
    Ok(true)
}

/// IDA: 00451160 TESFileData_SeekRecordChunks: +0x260, +0x264 = 0; unless the record is compressed (not a GRUP, +0x244 bit 0x40000) the seek to +0x25C + 0x14; +0x250, +0x254 = 0; a tail call of 00450F10. Completion: 100%.
/// # Errors
/// A read failure in the host.
pub fn seek_record_chunks(
    file: &mut TesFileData,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
    file.chunk_offset_260 = 0;
    file.fetched_264 = 0;
    let target = file.record_offset_25c.wrapping_add(0x14);
    if !compressed_record(file) {
        seek_file(file, target)?;
    }
    file.chunk_type_250 = 0;
    file.chunk_size_254 = 0;
    read_next_chunk_header(file, services)
}

/// IDA: 00451580 TESFileData_SkipGroup: a GRUP clears its type (+0x23C = 0), takes 0x14 off its size and tail-calls 00451310, whose record arm adds size + 0x14, so the whole group goes. Anything else returns 0, with eax already 0. Completion: 100%.
pub fn skip_group(
    files: &mut TesFileStore,
    id: TesFileId,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
    let file = files.get_mut(id)?;
    if file.record_23c.type_23c != GRUP_FOURCC {
        return Ok(false);
    }
    file.record_23c.type_23c = 0;
    file.record_23c.size_240 = file.record_23c.size_240.wrapping_add(0xFFFF_FFEC);
    next_form(files, id, services)
}

/// IDA: N/A the current record's reader, as a LoadForm holds it: the file by id, with the services 00450C20 and its neighbours warn through.
pub struct RecordReader<'a, 'b> {
    pub files: &'a mut TesFileStore,
    pub id: TesFileId,
    pub services: &'a mut TesFileServices<'b>,
}

impl RecordReader<'_, '_> {
    /// IDA: N/A the TESFile.
    pub fn file(&mut self) -> Result<&mut TesFileData, GameError> {
        self.files.get_mut(self.id)
    }

    /// IDA: 004511C0.
    pub fn record_form_type(&mut self) -> Result<u32, GameError> {
        current_record_form_type(self.files, self.id, self.services)
    }

    /// IDA: 004510B0.
    pub fn chunk_type(&mut self) -> Result<u32, GameError> {
        current_chunk_type(self.files.get_mut(self.id)?, self.services)
    }

    /// IDA: 0044FEA0.
    pub fn advance_chunk(&mut self) -> Result<bool, GameError> {
        advance_chunk(self.files.get_mut(self.id)?, self.services)
    }

    /// IDA: +0x254.
    pub fn chunk_size(&mut self) -> Result<u32, GameError> {
        Ok(self.file()?.chunk_size_254)
    }

    /// IDA: 00450C20.
    pub fn read_chunk(&mut self, destination: &mut [u8], size: u32) -> Result<bool, GameError> {
        read_current_chunk_data(
            self.files.get_mut(self.id)?,
            destination,
            size,
            self.services,
        )
    }

    /// IDA: `alloca(+0x254)`, then 00450C20(size): a chunk's text with the limit `size` (0 is none). The host buffer holds a NUL past the chunk, where a chunk with no terminator runs on into the stack in the original. A short read's result is ignored, as every caller does.
    pub fn read_text(&mut self, size: u32) -> Result<Vec<u8>, GameError> {
        let length = self.chunk_size()? as usize;
        let mut buffer = vec![0_u8; length + 1];
        self.read_chunk(&mut buffer, size)?;
        Ok(c_bytes(&buffer).to_vec())
    }

    /// IDA: 004510E0 TESFileData_ReadChunk4 (00450C20 with size 4), over a dword `current`, which an empty chunk leaves and a short one part-fills.
    pub fn read_dword(&mut self, current: u32) -> Result<(u32, bool), GameError> {
        let mut bytes = current.to_le_bytes();
        let read = self.read_chunk4(&mut bytes)?;
        Ok((u32::from_le_bytes(bytes), read))
    }

    // IDA: N/A 004510E0's call. A chunk of 4 bytes or less fits the array: a bigger one takes the size-limited arm.
    fn read_chunk4(&mut self, bytes: &mut [u8; 4]) -> Result<bool, GameError> {
        self.read_chunk(bytes, 4)
    }

    /// IDA: +0x25C the current record's file offset.
    pub fn record_offset(&mut self) -> Result<u32, GameError> {
        Ok(self.file()?.record_offset_25c)
    }
}

// IDA: 004515B0 TESFileData_OpenFile(directory, name, openMode, forceModeOne). Completion: 100% of the error states 2 / 12 / 9, the find-data comparison (flags |= 2 and
// copy on change), the BSFile open and the 004512A0 reset. Only modes 0 and 1 are reached.
pub fn open_tes_file(
    files: &mut TesFileStore,
    id: TesFileId,
    directory: &str,
    name: &str,
    open_mode: u32,
    force_mode_one: bool,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
    let open_mode = if force_mode_one { 1 } else { open_mode };
    let file = files.get_mut(id)?;
    // 00451624..0045165B the path (directory + name), +0x120 and +0x1C.
    let path = format!("{directory}{name}");
    file.directory_120 = directory.to_owned();
    file.name_1c = name.to_owned();
    if file.bs_file_10.is_none() {
        let Some(found) = find_all(services.working_directory, &path)
            .into_iter()
            .next()
        else {
            // 00451684..00451741 the ".tes" spelling: found -> 12, else 2.
            let stem = file.name_1c.split('.').next().unwrap_or_default();
            let spelled = format!("{}{stem}.tes", file.directory_120);
            file.error_state_00 = if find_all(services.working_directory, &spelled).is_empty() {
                2
            } else {
                0x0C
            };
            return Ok(false);
        };
        // 0045174A..004517EB changed find data: flags |= 2 and copy.
        if file.find_data_290 != found {
            file.flags_3dc |= 2;
            file.find_data_290 = found;
        }
        // 004517ED..00451841 BSFile(path, mode, +0x228, 0) -> +0x10.
        let mode = match open_mode {
            0 => NiFileMode::ReadOnly,
            1 => NiFileMode::WriteOnly,
            _ => {
                return Err(GameError::unclassified(
                    0x004517ED,
                    "004517ED: a TESFile open mode above 1",
                ));
            }
        };
        let mut stream = BsFile::new(
            services.working_directory,
            &path,
            mode,
            file.buffer_size_228,
        );
        // 00451847 vt+0x18 Open(1, 0); 00451857 +0x24 good test. errno is the host's, with the same values for ENOENT and EACCES.
        stream.open();
        let good = stream.good();
        let error = std::io::Error::last_os_error().raw_os_error();
        file.bs_file_10 = Some(stream);
        if !good {
            match error {
                Some(2) => file.error_state_00 = 2,
                Some(13) => file.error_state_00 = 9,
                _ => {}
            }
            return Ok(false);
        }
    }
    // 00451881 +0x258 = vt+0x1C size; 0045189C 004512A0(mode == 0).
    file.file_size_258 = file.bs_file_10.as_mut().map_or(0, BsFile::compute_size);
    rewind_tes_file(files, id, open_mode == 0, services)?;
    Ok(true)
}

// IDA: N/A a C string's bytes up to its NUL.
#[must_use]
pub fn c_bytes(bytes: &[u8]) -> &[u8] {
    bytes.split(|&byte| byte == 0).next().unwrap_or(bytes)
}

// IDA: 00451A60 TESFileData_ReadHeader: 0 on success, 2 when the open fails, 0xB when the first record is not TES4. Completion: 100% of the CNAM / SNAM / DATA / HEDR / MAST
// handling, the record-flag copies, the skip over TES4 records and the final rewind.
pub fn read_tes_file_header(
    files: &mut TesFileStore,
    id: TesFileId,
    services: &mut TesFileServices<'_>,
) -> Result<u32, GameError> {
    let file = files.get_mut(id)?;
    // 00451A72 0044FBC0 TESFile_ClearMasterLists.
    file.masters_3e0.clear();
    file.master_sizes_3e8.clear();
    file.master_count_3f0 = 0;
    if file.bs_file_10.is_none() {
        // 00451A80..00451A96 open (mode 0): failure returns 2.
        let (directory, name) = (file.directory_120.clone(), file.name_1c.clone());
        if !open_tes_file(files, id, &directory, &name, 0, false, services)? {
            return Ok(2);
        }
    } else {
        // 00451AB0..00451AF2 rewind without the header read, then 004511C0.
        seek_file(file, 0)?;
        file.record_offset_25c = 0;
        file.chunk_offset_260 = 0;
        file.fetched_264 = 0;
        clear_record(file);
        current_record_form_type(files, id, services)?;
    }
    let file = files.get_mut(id)?;
    // 00451AF7 +0x258 = vt+0x1C size.
    file.file_size_258 = file.bs_file_10.as_mut().map_or(0, BsFile::compute_size);
    // 00451B07 the first record must be TES4 (00B05E14).
    if file.record_23c.type_23c != TES4_FOURCC {
        return Ok(0x0B);
    }
    // 00451B30..00451C68 the chunk loop.
    loop {
        let chunk = file.chunk_type_250;
        if chunk == CNAM_FOURCC || chunk == SNAM_FOURCC {
            let mut buffer = vec![0_u8; (file.chunk_size_254 as usize).max(1) + 0x200];
            read_current_chunk_data(file, &mut buffer, 0x200, services)?;
            *buffer.last_mut().expect("nonempty") = 0;
            let target = if chunk == CNAM_FOURCC {
                &mut file.author_404
            } else {
                &mut file.description_40c
            };
            target.set(Some(c_bytes(&buffer)), 0);
        } else if chunk == DATA_FOURCC {
            let mut block = vec![0_u8; (file.chunk_size_254 as usize).max(8)];
            read_current_chunk_data(file, &mut block, 8, services)?;
            file.master_sizes_3e8
                .push(block[..8].try_into().expect("eight bytes"));
        } else if chunk == HEDR_FOURCC {
            // 00451C1C reads straight into +0x3D0, so a short chunk keeps the old words.
            let mut block = vec![0_u8; (file.chunk_size_254 as usize).max(0x0C)];
            for (at, word) in file.header_3d0.into_iter().enumerate() {
                block[at * 4..at * 4 + 4].copy_from_slice(&word.to_le_bytes());
            }
            read_current_chunk_data(file, &mut block, 0x0C, services)?;
            file.header_3d0 = [dword(&block, 0), dword(&block, 4), dword(&block, 8)];
            // 00451C2C..00451C59 next form ID: at least 0x800, mod index in the top byte.
            if file.header_3d0[2] <= 0x7FF {
                file.header_3d0[2] = 0x800;
            }
            file.header_3d0[2] =
                (u32::from(file.mod_index_400) << 24) | (file.header_3d0[2] & 0x00FF_FFFF);
        } else if chunk == MAST_FOURCC {
            let mut text = vec![0_u8; file.chunk_size_254 as usize];
            read_current_chunk_data(file, &mut text, 0, services)?;
            file.masters_3e0.push(c_bytes(&text).to_vec());
            file.master_count_3f0 += 1;
        }
        if !advance_chunk(file, services)? {
            break;
        }
    }
    // 00451C6E..00451CC7 the record flags: 1 -> master, 0x10, 0x80; the top byte of +0x3DC is moved out and back (no net change).
    let flags = file.record_23c.flags_244;
    for bit in [1_u32, 0x10, 0x80] {
        if flags & bit != 0 {
            file.flags_3dc |= bit;
        } else {
            file.flags_3dc &= !bit;
        }
    }
    // 00451CD0..00451D12 skip every further TES4 record.
    loop {
        if files.get_mut(id)?.record_23c.type_23c == 0 {
            if !read_next_record_header(files, id)? {
                break;
            }
            let file = files.get_mut(id)?;
            file.chunk_type_250 = 0;
            file.chunk_size_254 = 0;
            read_next_chunk_header(file, services)?;
        }
        let kind = files.get_mut(id)?.record_23c.type_23c;
        if form_type_from_record_fourcc(kind, services.form_type_cache) != 1 {
            break;
        }
        skip_record(files, id, true, services)?;
    }
    // 00451D14..00451D2E header read when the next record is a group or none.
    let next = current_record_form_type(files, id, services)? & 0xFF;
    files.get_mut(id)?.header_read_28c = u8::from(next == 2 || next == 0);
    // 00451D29..00451D78 rewind, then 004511C0.
    rewind_tes_file(files, id, true, services)?;
    Ok(0)
}

// IDA: 004507D0 TESFileData_Close: returns true at once when the DataHandler suppresses closing; otherwise drops the open-group list, deletes the BSFile and frees the
// decompressed buffer (a +0x0C write file stops: not reached). Completion: 100%.
pub fn close_tes_file(
    file: &mut TesFileData,
    services: &TesFileServices<'_>,
) -> Result<bool, GameError> {
    // 004507E4..004507FF the DataHandler suppresses closing.
    if services.close_suppressed {
        return Ok(true);
    }
    // 00450815 00450430 open-group cleanup (empty here); 0045081A..00450829 delete the BSFile; 0045082C..00450847 the decompressed buffer.
    file.open_groups_284 = [0; 2];
    file.bs_file_10 = None;
    file.decompressed_414 = None;
    file.decompressed_length_418 = 0;
    if file.file_0c != 0 {
        return Err(GameError::unclassified(
            0x00450851,
            "00450851: the TESFile write path is not reached at startup",
        ));
    }
    Ok(true)
}

// IDA: 00451DA0 TESFileData_ctor(directory, name, openMode). Completion: 100% of the store order and the open / header-read / close calls and their warnings.
pub fn construct_tes_file(
    files: &mut TesFileStore,
    directory: &str,
    name: &str,
    open_mode: u32,
    buffer_size: u32,
    services: &mut TesFileServices<'_>,
) -> Result<TesFileId, GameError> {
    // 00451DCC..00451F0E the constructor stores (all zero except +0x400 = 0xFF, +0x228 = *(00B055CC), +0x3D8 = 0x800; +0x290 memset 0).
    let id = files.push(TesFileData {
        mod_index_400: 0xFF,
        buffer_size_228: buffer_size,
        header_3d0: [0, 0, 0x800],
        ..TesFileData::default()
    });
    // 00451F18 004515B0(directory, name, mode, 0).
    if open_tes_file(files, id, directory, name, open_mode, false, services)? {
        let found = &files.get_mut(id)?.find_data_290;
        // 00451F21..00451F47 a non-empty file with a failed header read warns.
        if (found.size_high != 0 || found.size_low != 0)
            && read_tes_file_header(files, id, services)? != 0
        {
            services.warn(format!("File '{name}' is not a valid TES file."));
        }
        // 00451F4A..00451F5B close; a failure warns.
        if !close_tes_file(files.get_mut(id)?, services)? {
            services.warn(format!("Could not close file '{name}'."));
        }
    }
    Ok(id)
}

// IDA: 0044FC70 TESFile_ResolveMasters(fileList, warn): frees +0x3F4 and, with masters, allocates one slot per master and resolves each name (_stricmp) against the list; a
// missing master stores 0 (warning "Missing Masterfile: %s" when warn) and the result becomes false. Completion: 100%.
pub fn resolve_tes_file_masters(
    id: TesFileId,
    list: &[TesFileId],
    store: &mut TesFileStore,
    warn: bool,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
    let names: Vec<(TesFileId, String)> = list
        .iter()
        .map(|&entry| Ok((entry, store.get_mut(entry)?.name_1c.clone())))
        .collect::<Result<_, GameError>>()?;
    let file = store.get_mut(id)?;
    // 0044FC75..0044FCA1 free +0x3F4; no masters returns true.
    file.master_files_3f4.clear();
    file.master_files_allocated = false;
    if file.master_count_3f0 == 0 {
        return Ok(true);
    }
    // 0044FCAC..0044FCC9 one slot per master (unset slots stay 0 here).
    file.master_files_3f4 = vec![None; file.master_count_3f0 as usize];
    file.master_files_allocated = true;
    let mut result = true;
    for slot in 0..file.masters_3e0.len() {
        let master = &file.masters_3e0[slot];
        match names
            .iter()
            .find(|(_, name)| stricmp(master, &host_path::original_bytes(name)) == 0)
        {
            Some((entry, _)) => file.master_files_3f4[slot] = Some(*entry),
            None => {
                if warn {
                    let text = String::from_utf8_lossy(master).into_owned();
                    services.warn(format!("Missing Masterfile: {text}"));
                }
                result = false;
            }
        }
    }
    Ok(result)
}

// IDA: 0044A2B0 DataHandler_BuildFileList(list +0x8C8, directory). Completion: 100% of the order of the five passes (existing entries, *.esm then *.esp, missing files,
// master reorder, master resolution, closes). `list` is the BSSimpleList in order: the original has no holes after 0044A2B0's edits, so a `Vec` keeps its node semantics.
pub fn build_tes_file_list(
    list: &mut Vec<TesFileId>,
    store: &mut TesFileStore,
    directory: &str,
    buffer_size: u32,
    services: &mut TesFileServices<'_>,
) -> Result<(), GameError> {
    // 0044A307..0044A367 re-read the existing entries; a failed header read drops the entry from the list (the TESFile itself is not freed).
    let mut index = 0;
    while index < list.len() {
        let id = list[index];
        if read_tes_file_header(store, id, services)? != 0 {
            list.remove(index);
        } else {
            index += 1;
        }
        close_tes_file(store.get_mut(id)?, services)?;
    }
    // 0044A369..0044A54B "*.esm" then "*.esp".
    for extension in ["*.esm", "*.esp"] {
        let pattern = format!("{directory}{extension}");
        for found in find_all(services.working_directory, &pattern) {
            if found.size_high == 0 && found.size_low == 0 {
                continue;
            }
            // 0044A3E4..0044A416 an entry with the same name is kept.
            let mut present = false;
            for &id in list.iter() {
                if stricmp(
                    found.file_name.as_bytes(),
                    store.get_mut(id)?.name_1c.as_bytes(),
                ) == 0
                {
                    present = true;
                    break;
                }
            }
            if present {
                continue;
            }
            // 0044A41C..0044A45D FormHeap(0x41C) TESFileData(directory, name, 0), then 004507D0.
            let created_id =
                construct_tes_file(store, directory, &found.file_name, 0, buffer_size, services)?;
            let created = store.get_mut(created_id)?;
            close_tes_file(created, services)?;
            let created_master = created.flags_3dc & 1 != 0;
            // 0044A462..0044A50F the master-first, write-time ordered insert (0044A47C 0044FAA0 TESFile_IsMaster: +0x3DC & 1).
            let mut inserted = false;
            for position in 0..list.len() {
                let existing = store.get_mut(list[position])?;
                let existing_master = existing.flags_3dc & 1 != 0;
                // CompareFileTime(existing, found) >= 0.
                let insert_here = if existing_master == created_master {
                    existing.find_data_290.last_write_time >= found.last_write_time
                } else {
                    created_master
                };
                if insert_here {
                    list.insert(position, created_id);
                    inserted = true;
                    break;
                }
            }
            if !inserted {
                list.push(created_id);
            }
        }
    }
    // 0044A551..0044A611 drop the entries whose file is gone.
    let mut index = 0;
    while index < list.len() {
        let id = list[index];
        let file = store.get_mut(id)?;
        let path = format!("{}{}", file.directory_120, file.name_1c);
        if !find_all(services.working_directory, &path).is_empty() {
            index += 1;
            continue;
        }
        list.remove(index);
        // 0044A5F1 00450AE0 destructor and FormHeap_Free.
        close_tes_file(file, services)?;
        store.files[id.0] = None;
    }
    // 0044A617..0044A6D5 move each master file's own masters ahead of it (0044A657 0044FDD0 TESFile_GetMasterCount: +0x3F0).
    let mut index = 0;
    while index < list.len() {
        let id = list[index];
        let mut moved = false;
        if store.get_mut(id)?.flags_3dc & 1 != 0 {
            resolve_tes_file_masters(id, list, store, false, services)?;
            let count = store.get_mut(id)?.master_count_3f0;
            for master_index in 1..=count {
                let Some(master) = master_at(store.get_mut(id)?, id, master_index) else {
                    continue;
                };
                // 0044A67C the search runs from this node on, and 0065C620 removes that match; the node then takes the master and its old item moves to a new next node.
                if let Some(offset) = list[index..].iter().position(|&entry| entry == master) {
                    list.remove(index + offset);
                    list.insert(index, master);
                    moved = true;
                }
            }
        }
        if !moved {
            index += 1;
        }
    }
    // 0044A6DB..0044A6FC resolve every file's masters.
    for position in 0..list.len() {
        resolve_tes_file_masters(list[position], list, store, false, services)?;
    }
    // 0044A6FE..0044A714 close every file.
    for &id in list.iter() {
        close_tes_file(store.get_mut(id)?, services)?;
    }
    Ok(())
}

// ---- Phase 10: the +0x3DC selection flags (xOBSE GameData.h `ModEntry::Data`). ----

/// IDA: TESFile +0x3DC bit 0, `kFlag_IsMaster` (00451C6E copies it from the TES4 record flags).
pub const TES_FILE_FLAG_IS_MASTER: u32 = 0x1;
/// IDA: TESFile +0x3DC bit 2, `kFlag_Loaded`. Its first reader is 0044F3D0 DataHandler_LoadSelectedFiles (Phases 11 and 12).
pub const TES_FILE_FLAG_LOADED: u32 = 0x4;
/// IDA: TESFile +0x3DC bit 3, `kFlag_Active`.
pub const TES_FILE_FLAG_ACTIVE: u32 = 0x8;

impl TesFileData {
    // IDA: 0044FAD0 TESFile_SetLoaded(this, bool) (__thiscall, retn 4): a nonzero argument byte ORs 4 into +0x3DC (0044FAD7); zero ANDs 0xFFFFFFF3 (0044FAE1), clearing Loaded and Active. Nothing else is
    // stored. Completion: 100%.
    pub const fn set_loaded(&mut self, loaded: bool) {
        if loaded {
            self.flags_3dc |= TES_FILE_FLAG_LOADED;
        } else {
            self.flags_3dc &= !(TES_FILE_FLAG_LOADED | TES_FILE_FLAG_ACTIVE);
        }
    }
}

impl TesFileData {
    /// IDA: 0044FAA0 TESFile_IsMaster (+0x3DC & 1). Completion: 100%.
    #[must_use]
    pub const fn is_master(&self) -> bool {
        self.flags_3dc & TES_FILE_FLAG_IS_MASTER != 0
    }

    /// IDA: 0044FAC0 TESFile_IsLoaded (+0x3DC bit 2). Completion: 100%.
    #[must_use]
    pub const fn is_loaded(&self) -> bool {
        self.flags_3dc & TES_FILE_FLAG_LOADED != 0
    }

    /// IDA: 0044FAF0 TESFile_IsActive (+0x3DC bit 3). Completion: 100%.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.flags_3dc & TES_FILE_FLAG_ACTIVE != 0
    }

    /// IDA: 0044FB20 TESFile_SetModIndex(index): +0x3D8 = (+0x3D8 & 0xFFFFFF) | index << 24, byte +0x400 = index. Completion: 100%.
    pub const fn set_mod_index(&mut self, index: u8) {
        self.header_3d0[2] = (self.header_3d0[2] & 0x00FF_FFFF) | ((index as u32) << 24);
        self.mod_index_400 = index;
    }

    /// IDA: 0044FD90 TESFile_GetMasterName(index): the MAST text at 1-based `index` (0 and 1 both give the first); null for an empty list or past the end. Completion: 100%.
    #[must_use]
    pub fn master_name(&self, index: u32) -> Option<&[u8]> {
        self.masters_3e0
            .get((index.max(1) - 1) as usize)
            .map(Vec::as_slice)
    }

    /// IDA: 0044FDE0 TESFile_MastersChanged: walks the MAST names (+0x3E0) with the DATA sizes (+0x3E8) until a name, its size node or its block is missing. For each, FindFirstFileA on the name alone, so relative to the process directory, not Data\\. The found
    /// {low, high} size is compared with the stored 8 bytes, or the stored bytes with 0 when nothing is found; `bl` stays set from any difference and the walk goes on. D6 / host substitution: `find_all` under the working directory. Completion: 100%.
    #[must_use]
    pub fn masters_changed(&self, working_directory: &std::path::Path) -> bool {
        let mut changed = false;
        for (name, size) in self.masters_3e0.iter().zip(&self.master_sizes_3e8) {
            let stored = u64::from_le_bytes(*size);
            let name: String = name.iter().map(|&byte| char::from(byte)).collect();
            let found = find_all(working_directory, &name).into_iter().next();
            let on_disk = found.map_or(0, |found| {
                (u64::from(found.size_high) << 32) | u64::from(found.size_low)
            });
            if on_disk != stored {
                changed = true;
            }
        }
        changed
    }
}

// IDA: 004510E0 into a local cleared first, as most arms do.
pub(crate) fn read_dword(cx: &mut FormLoadCx<'_, '_>) -> Result<u32, GameError> {
    Ok(cx.file.read_dword(0)?.0)
}

/// IDA: 0046BB20 over the world's files. Completion: 100% of `map_form_id_in`'s arms.
pub fn map_id(world: &World, id: u32, file: Option<TesFileId>) -> Result<u32, GameError> {
    let files = &world.resource::<DataHandler>().file_store;
    map_form_id_in(world, files, id, file)
}

// IDA: 004510E0 into a zeroed local, as most form-ID arms do.
pub(crate) fn read_id(cx: &mut FormLoadCx<'_, '_>) -> Result<u32, GameError> {
    Ok(cx.file.read_dword(0)?.0)
}

// IDA: 004510F0 (00450C20 with size 2) into a zeroed dword local, of which the low word goes on.
pub(crate) fn read_word(cx: &mut FormLoadCx<'_, '_>) -> Result<u16, GameError> {
    let mut bytes = [0_u8; 4];
    cx.file.read_chunk(&mut bytes, 2)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

// IDA: 0046BB2C g_saveLoadGame +0x18 bit 17.
const SAVE_LOAD_REMAP: u32 = 0x2_0000;

/// IDA: 0046BB20 TESFile_MapFormID(&id, file) (__cdecl). It reads g_saveLoadGame +0x18 first (null faults: a host stop); bit 17 maps through 00452180 (a host stop, not at startup). An ID in 1..=0x7FF stays; a null file leaves it; otherwise the top byte becomes the mod index (0044FB50, +0x400) of master `top + 1` (0044FD60), or the file's own when there is no such master. Completion: 100% of the arms but 00452180.
/// TODO: [ ] 0046BB38 00452180, with the save loader.
pub fn map_file_form_id(
    world: &World,
    cx: &FormLoadCx<'_, '_>,
    id: u32,
    file: Option<TesFileId>,
) -> Result<u32, GameError> {
    map_form_id_in(world, cx.file.files, id, file)
}

/// IDA: 0046BB20 over a store of files: `map_file_form_id`'s body, for a caller that holds them itself. Completion: 100% of the arms but 00452180.
pub fn map_form_id_in(
    world: &World,
    files: &TesFileStore,
    id: u32,
    file: Option<TesFileId>,
) -> Result<u32, GameError> {
    let flags = world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map(|game| game.fields_14[1])
        .ok_or(GameError::unclassified(
            0x0046BB26,
            "0046BB26: g_saveLoadGame is null",
        ))?;
    if flags & SAVE_LOAD_REMAP != 0 {
        return Err(GameError::unclassified(
            0x0046BB38,
            "0046BB38: 00452180 (the save-load remap)",
        ));
    }
    if id != 0 && id <= 0x7FF {
        return Ok(id);
    }
    let Some(file) = file else {
        return Ok(id);
    };
    let data = files.get(file).ok_or(GameError::unclassified(
        0x0046BB64,
        "0046BB64: a null TESFile faults",
    ))?;
    let source = master_at(data, file, (id >> 24) + 1)
        .and_then(|master| files.get(master))
        .unwrap_or(data);
    Ok((id & 0x00FF_FFFF) | (u32::from(source.mod_index_400) << 24))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 0044FCFE compares the MAST bytes with +0x1C, both in the ANSI code page. `name_1c` is a lifted String, so byte F6 is U+00F6, and its UTF-8 form must not reach the comparison.
    #[test]
    fn a_master_with_a_non_ascii_name_resolves() {
        let mut store = TesFileStore::default();
        let master = store.push(TesFileData {
            name_1c: "M\u{F6}d.esm".to_owned(),
            ..TesFileData::default()
        });
        let dependant = store.push(TesFileData {
            name_1c: "Dependant.esp".to_owned(),
            masters_3e0: vec![b"M\xF6D.ESM".to_vec()],
            master_count_3f0: 1,
            ..TesFileData::default()
        });
        let mut warnings = Vec::new();
        let mut warning = |text: String, _disabled: bool| warnings.push(text);
        let mut cache = FormTypeLookupCache::default();
        let mut services = TesFileServices {
            working_directory: std::path::Path::new("."),
            warning: &mut warning,
            disable_warning_messages_b06b18: 0,
            close_suppressed: true,
            form_type_cache: &mut cache,
        };
        let list = [master, dependant];
        assert!(
            resolve_tes_file_masters(dependant, &list, &mut store, true, &mut services).unwrap()
        );
        assert_eq!(
            store.get(dependant).unwrap().master_files_3f4,
            [Some(master)]
        );
    }
}
