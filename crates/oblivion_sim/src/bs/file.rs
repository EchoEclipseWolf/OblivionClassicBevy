//! NiFile, BSFile, ArchiveFile and CompressedArchiveFile. Field comments give original offsets.
//!
//! Recorded host substitutions: `std::fs::File` for the CRT `FILE*`; flate2 for the original's zlib
//! 1.2.1 (inflate results are defined by the format, so its output cannot differ).

use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::gamebryo::file::{FileCore, SEEK_CUR, SEEK_END, ni_file_swap_components};
use flate2::{Decompress, FlushDecompress, Status};
use oblivion_core::host_path;
use oblivion_core::logging::{Level, emit};

pub const SEEK_SET: i32 = 0;

const BS_FILE_PATH_LIMIT: usize = 0x104;

/// IDA: 00B04360 the archive-file buffer limit: image value 2 MiB, 128 KiB after its setter.
pub const ARCHIVE_FILE_BUFFER_LIMIT_IMAGE: u32 = 0x20_0000;
pub const ARCHIVE_FILE_BUFFER_LIMIT_128K: u32 = 0x2_0000;

/// IDA: 00747C70 NiFile mode: "rb", "wb", "ab".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum NiFileMode {
    ReadOnly = 0,
    WriteOnly = 1,
    AppendOnly = 2,
}

// IDA: N/A a helper, with no address of its own.
fn open_file(working: &Path, path: &str, options: &OpenOptions) -> Option<File> {
    if path.is_empty() {
        return None;
    }
    let file = options.open(host_path::resolve(working, path)).ok()?;
    // fopen on a directory fails on Windows; open succeeds on Linux.
    file.metadata().ok()?.is_file().then_some(file)
}

// IDA: N/A a helper, with no address of its own.
fn read_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true);
    options
}

// IDA: N/A a helper, with no address of its own.
fn c_string(value: &str) -> &str {
    value.split('\0').next().unwrap_or(value)
}

/// What every original file object answers, with the read and write the NiFile vtable routes.
pub trait GameFile {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn core(&self) -> &FileCore;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn core_mut(&mut self) -> &mut FileCore;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn proc_read(&mut self, buffer: &mut [u8]) -> usize;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn proc_write(&mut self, source: &[u8]) -> usize;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn seek(&mut self, offset: i32, origin: i32);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn size(&mut self) -> u32;
    /// IDA: BSFile types need a component list to swap; NiFile swaps with whatever it is given.
    fn swap_requires_components(&self) -> bool {
        true
    }

    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn good(&self) -> bool {
        self.core().good
    }
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn set_endian_swap(&mut self, swap: bool) {
        self.core_mut().swap = swap;
    }
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn buffer_size(&self) -> u32 {
        self.core().buffer_size
    }
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn buffer_valid(&self) -> u32 {
        self.core().buffer_valid
    }
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn has_handle(&self) -> bool {
        self.core().file.is_some()
    }

    /// IDA: the +0x04 read proc through the swap selection.
    fn read(&mut self, buffer: &mut [u8]) -> usize {
        self.read_components(buffer, None)
    }

    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn read_components(&mut self, buffer: &mut [u8], components: Option<&[u32]>) -> usize {
        if !self.core().swap {
            return self.proc_read(buffer);
        }
        if buffer.is_empty() {
            return 0;
        }
        let result = self.proc_read(buffer);
        if !self.swap_requires_components() || components.is_some() {
            ni_file_swap_components(buffer, components.unwrap_or(&[]));
        }
        result
    }

    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn write(&mut self, source: &[u8]) -> usize {
        self.proc_write(source)
    }

    /// IDA: 004308E0 ReadLine(buffer, max, delimiter): bytes until the delimiter (consumed, not
    /// stored), a short read or max - 1 stored. Returns (bytes consumed, the line).
    fn read_line(&mut self, max: u32, delimiter: u16) -> (u32, Vec<u8>) {
        let mut consumed = 0;
        let mut line = Vec::new();
        for _ in 1..max.max(1) {
            let mut byte = [0];
            let got = self.read_components(&mut byte, Some(&[1]));
            consumed += got as u32;
            if got != 1 || i32::from(byte[0] as i8) == i32::from(delimiter) {
                break;
            }
            line.push(byte[0]);
        }
        (consumed, line)
    }
}

/// IDA: 00747C70 NiFile (0x28): the default open callback's object. Completion: 100%.
#[derive(Debug)]
pub struct NiFile {
    core: FileCore,
}

impl NiFile {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn new(working: &Path, path: &str, mode: NiFileMode, buffer_size: u32) -> Self {
        let mut options = OpenOptions::new();
        match mode {
            NiFileMode::ReadOnly => options.read(true),
            NiFileMode::WriteOnly => options.write(true).create(true).truncate(true),
            NiFileMode::AppendOnly => options.append(true).create(true),
        };
        let file = open_file(working, c_string(path), &options);
        let mut core = FileCore {
            working_directory: working.to_path_buf(),
            buffer_size,
            mode: mode as u32,
            good: file.is_some(),
            file,
            ..FileCore::default()
        };
        if core.good && buffer_size > 0 {
            core.allocate_buffer(buffer_size);
        }
        Self { core }
    }
}

impl GameFile for NiFile {
    /// IDA: N/A `NiFile`'s side of `GameFile`.
    fn core(&self) -> &FileCore {
        &self.core
    }
    /// IDA: N/A `NiFile`'s side of `GameFile`.
    fn core_mut(&mut self) -> &mut FileCore {
        &mut self.core
    }
    /// IDA: N/A `NiFile`'s side of `GameFile`.
    fn proc_read(&mut self, buffer: &mut [u8]) -> usize {
        self.core.buffered_read(buffer)
    }
    /// IDA: N/A `NiFile`'s side of `GameFile`.
    fn proc_write(&mut self, source: &[u8]) -> usize {
        self.core.buffered_write(source)
    }
    /// IDA: N/A `NiFile`'s side of `GameFile`.
    fn seek(&mut self, offset: i32, origin: i32) {
        self.core.buffered_seek(offset, origin);
    }
    /// IDA: N/A `NiFile`'s side of `GameFile`.
    fn size(&mut self) -> u32 {
        self.core.file_size_from_handle()
    }
    /// IDA: N/A `NiFile`'s side of `GameFile`.
    fn swap_requires_components(&self) -> bool {
        false
    }
}

impl Drop for NiFile {
    /// IDA: N/A Rust's `Drop`.
    fn drop(&mut self) {
        if self.core.good && self.core.file.is_some() {
            self.core.flush();
        }
    }
}

/// IDA: 00430970 BSFile (0x154): +0x30 fixed position, +0x3C path, +0x148 position, +0x14C shadow,
/// +0x150 cached size. Completion: 100%; text mode is not reached.
#[derive(Debug)]
pub struct BsFile {
    core: FileCore,
    fixed_position_30: i32,
    path_3c: String,
    position_148: u32,
    position_shadow_14c: u32,
    cached_size_150: u32,
}

impl BsFile {
    /// IDA: 00430970: the +0x3C copy keeps fewer than 0x104 characters. Mode 1 opens at once; mode 0
    /// takes +0x24 from `_access` without opening.
    #[must_use]
    pub fn new(working: &Path, path: &str, mode: NiFileMode, buffer_size: u32) -> Self {
        let mut file = Self {
            core: FileCore {
                working_directory: working.to_path_buf(),
                buffer_size,
                mode: mode as u32,
                ..FileCore::default()
            },
            fixed_position_30: -1,
            path_3c: String::new(),
            position_148: 0,
            position_shadow_14c: 0,
            cached_size_150: 0,
        };
        let source = c_string(path);
        if host_path::original_length(source) < BS_FILE_PATH_LIMIT {
            file.path_3c = source.to_owned();
            if mode == NiFileMode::WriteOnly {
                file.open();
                return file;
            }
        }
        // 00430A28: only mode 0 initializes +0x24 here.
        if mode == NiFileMode::ReadOnly {
            file.core.good = host_path::access(working, &file.path_3c);
        }
        file
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path_3c
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn position(&self) -> u32 {
        self.position_148
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn position_shadow(&self) -> u32 {
        self.position_shadow_14c
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn fixed_position(&self) -> i32 {
        self.fixed_position_30
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn cached_size(&self) -> u32 {
        self.cached_size_150
    }
    /// IDA: +0x30 when set, else +0x148.
    #[must_use]
    pub const fn effective_position(&self) -> u32 {
        if self.fixed_position_30 == -1 {
            self.position_148
        } else {
            self.fixed_position_30 as u32
        }
    }

    /// IDA: 0042FE80 Open: "rb", or "r+b" with a create on miss; a whole-file buffer for size -1.
    pub fn open(&mut self) -> bool {
        if self.core.file.is_some() {
            return true;
        }
        self.core.buffer_position = 0;
        self.core.buffer_valid = 0;
        self.core.buffer = None;
        let working = self.core.working_directory.clone();
        let mut options = read_options();
        if self.core.mode != 0 {
            options.write(true);
        }
        self.core.file = open_file(&working, &self.path_3c, &options);
        if self.core.file.is_none() && self.core.mode == 1 {
            let mut create = OpenOptions::new();
            create.write(true).create(true).truncate(true);
            drop(open_file(&working, &self.path_3c, &create));
            self.core.file = open_file(&working, &self.path_3c, &options);
        }
        if self.core.file.is_none() {
            self.core.good = false;
            return false;
        }
        self.core.good = true;
        if self.core.buffer_size != 0 && self.core.buffer.is_none() {
            let whole_file = self.core.buffer_size == u32::MAX;
            if whole_file {
                self.core.buffer_size = self.size();
            }
            let size = self.core.buffer_size;
            self.core.allocate_buffer(size);
            if whole_file {
                self.core.buffer_valid = size;
                self.core.buffer_position = 0;
                let mut buffer = self.core.buffer.take().expect("just allocated");
                if self.core.raw_read(&mut buffer[..size as usize]) != size as usize {
                    self.core.good = false;
                }
                self.core.buffer = Some(buffer);
            }
        }
        self.core.good
    }

    /// IDA: 004301E0 vt+0x1C ComputeSize.
    pub fn compute_size(&mut self) -> u32 {
        self.cached_size_150 = 0;
        if self.core.mode == 1 {
            self.core.flush();
        }
        self.open();
        self.cached_size_150 = self.core.file_size_from_handle();
        self.cached_size_150
    }

    /// IDA: 00430700 ReadString: bytes to a NUL through 259-byte chunks; a total past `max` ends it.
    /// Returns the bytes consumed.
    pub fn read_string(&mut self, out: &mut Vec<u8>, max: u32) -> u32 {
        if self.core.file.is_none() {
            self.open();
        }
        out.clear();
        let mut consumed = 0;
        loop {
            let mut byte = [0];
            let got = self.read_components(&mut byte, Some(&[1]));
            consumed += got as u32;
            let value = if got == 1 && consumed <= max {
                byte[0]
            } else {
                0
            };
            if value == 0 {
                return consumed;
            }
            out.push(value);
        }
    }

    /// IDA: 00430AB0 the wide form, in 16-bit units; it returns units consumed.
    pub fn read_wstring(&mut self, out: &mut Vec<u16>, max: u32) -> u32 {
        if self.core.file.is_none() {
            self.open();
        }
        out.clear();
        let mut consumed = 0;
        loop {
            let mut unit = [0; 2];
            let got = self.read_components(&mut unit, Some(&[1]));
            consumed += got as u32;
            let value = if got == 2 && consumed <= max {
                u16::from_le_bytes(unit)
            } else {
                0
            };
            if value == 0 {
                return consumed >> 1;
            }
            out.push(value);
        }
    }

    /// IDA: 00430880 WriteString: the text and its NUL. +0x148 counts these bytes twice, in the read
    /// proc and here. Returns the bytes written.
    pub fn write_string(&mut self, data: &[u8]) -> u32 {
        let mut with_nul = data.to_vec();
        with_nul.push(0);
        let written = self.write(&with_nul) as u32;
        self.position_148 += written;
        written
    }

    /// IDA: 00430820 the wide form; it returns units written.
    pub fn write_wstring(&mut self, data: &[u16]) -> u32 {
        let mut bytes: Vec<u8> = data.iter().flat_map(|unit| unit.to_le_bytes()).collect();
        bytes.extend([0, 0]);
        let written = self.write(&bytes) as u32;
        self.position_148 += written;
        written >> 1
    }

    /// IDA: vt+0x34.
    #[must_use]
    pub fn exists(&self) -> bool {
        host_path::access(&self.core.working_directory, &self.path_3c)
    }

    /// IDA: 004303F0 SetBufferSize: rewinds, and leaves both positions at 0.
    pub fn set_buffer_size(&mut self, size: u32) -> bool {
        if size == self.core.buffer_size {
            return true;
        }
        self.core.buffered_seek(0, SEEK_SET);
        self.core.flush();
        self.core.buffer_size = size;
        self.core.allocate_buffer(size);
        self.position_148 = 0;
        self.position_shadow_14c = 0;
        true
    }
}

impl GameFile for BsFile {
    /// IDA: N/A `BsFile`'s side of `GameFile`.
    fn core(&self) -> &FileCore {
        &self.core
    }
    /// IDA: N/A `BsFile`'s side of `GameFile`.
    fn core_mut(&mut self) -> &mut FileCore {
        &mut self.core
    }
    /// IDA: 004302B0.
    fn proc_read(&mut self, buffer: &mut [u8]) -> usize {
        if self.core.file.is_none() {
            self.open();
        }
        let got = self.core.buffered_read(buffer);
        self.position_148 += got as u32;
        got
    }
    /// IDA: N/A `BsFile`'s side of `GameFile`.
    fn proc_write(&mut self, source: &[u8]) -> usize {
        let written = self.core.buffered_write(source);
        self.position_148 += written as u32;
        written
    }
    /// IDA: 0042FF80: a read-only SEEK_SET becomes relative to +0x148; a move to the current
    /// position does nothing.
    fn seek(&mut self, offset: i32, origin: i32) {
        let mut effective_origin = origin;
        let mut passed = offset;
        let mut target = self.position_148;
        if origin == SEEK_SET {
            target = offset as u32;
            if self.core.mode == 0 {
                effective_origin = SEEK_CUR;
                passed = offset.wrapping_sub(self.position_148 as i32);
            }
        } else if origin == SEEK_CUR {
            target = self.position_148.wrapping_add(offset as u32);
        } else if origin == SEEK_END {
            target = self.compute_size().wrapping_sub(offset as u32);
        }
        if target == self.position_148 {
            return;
        }
        self.core.buffered_seek(passed, effective_origin);
        self.position_148 = target;
    }
    /// IDA: N/A `BsFile`'s side of `GameFile`.
    fn size(&mut self) -> u32 {
        if self.cached_size_150 == 0 {
            self.compute_size();
        }
        self.cached_size_150
    }
}

impl Drop for BsFile {
    /// IDA: N/A Rust's `Drop`.
    fn drop(&mut self) {
        if self.core.good && self.core.file.is_some() {
            self.core.flush();
        }
    }
}

/// What an open ArchiveFile shares with its Archive: the BSFile base with its lock (+0x200) and the
/// +0x1A8 open count.
#[derive(Debug)]
pub struct ArchiveShared {
    pub stream: Mutex<BsFile>,
    open_count_1a8: AtomicI32,
    buffer_limit: Arc<AtomicU32>,
}

impl ArchiveShared {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn new(stream: BsFile, buffer_limit: Arc<AtomicU32>) -> Self {
        Self {
            stream: Mutex::new(stream),
            open_count_1a8: AtomicI32::new(0),
            buffer_limit,
        }
    }

    /// IDA: +0x1A8 InterlockedIncrement (0042E09D, 0042D5AF).
    pub fn acquire(&self) {
        self.open_count_1a8.fetch_add(1, Ordering::SeqCst);
    }

    /// IDA: 0042C910 Archive_Release: the decrement. The flag-0x02 delete branch has no writer in
    /// this executable, so it is not ported. Completion: 100%.
    pub fn release(&self) {
        self.open_count_1a8.fetch_sub(1, Ordering::SeqCst);
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn open_count(&self) -> i32 {
        self.open_count_1a8.load(Ordering::SeqCst)
    }
}

/// IDA: 0042D540 ArchiveFile (0x15C): a record read through the owner's stream under its lock.
/// Completion: 100%.
#[derive(Debug)]
pub struct ArchiveFile {
    core: FileCore,
    path_3c: String,
    position_148: u32,
    cached_size_150: u32,
    owner_154: Arc<ArchiveShared>,
    offset_158: u32,
}

impl ArchiveFile {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn new(
        path: &str,
        owner: Arc<ArchiveShared>,
        offset: u32,
        size: u32,
        options: i32,
    ) -> Self {
        owner.acquire();
        let limit = owner.buffer_limit.load(Ordering::Relaxed);
        let mut buffer_size = size;
        if size > limit && options != -1 {
            buffer_size = limit;
        }
        let mut path_3c = c_string(path).to_owned();
        path_3c.truncate(BS_FILE_PATH_LIMIT - 1);
        let mut file = Self {
            core: FileCore {
                buffer_size,
                good: true,
                ..FileCore::default()
            },
            path_3c,
            position_148: 0,
            cached_size_150: size,
            owner_154: owner,
            offset_158: offset,
        };
        if buffer_size != 0 {
            file.core.allocate_buffer(buffer_size);
        }
        if options == -1 {
            file.load_window();
        }
        file
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path_3c
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn position(&self) -> u32 {
        self.position_148
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn cached_size(&self) -> u32 {
        self.cached_size_150
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn archive_offset(&self) -> u32 {
        self.offset_158
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn owner(&self) -> &Arc<ArchiveShared> {
        &self.owner_154
    }
    /// IDA: vt+0x34: the owner is non-null.
    #[must_use]
    pub const fn exists(&self) -> bool {
        true
    }

    // IDA: N/A a helper, with no address of its own.
    fn load_window(&mut self) {
        let mut buffer = self.core.buffer.take().unwrap_or_default();
        let size = (self.core.buffer_size as usize).min(buffer.len());
        self.core.buffer_valid = self.read_at(&mut buffer[..size], 0) as u32;
        self.core.buffer = Some(buffer);
    }

    /// IDA: 0042C3E0: seek the owner's stream to the record position, clamp to +0x150 (a start past
    /// it would wrap in the original; the host reads nothing), read, and set its +0x148 / +0x14C.
    fn read_at(&mut self, destination: &mut [u8], extra: u32) -> usize {
        let mut stream = self.owner_154.stream.lock().expect("the archive lock");
        let target = self.offset_158 + self.position_148 + extra;
        let current = stream.effective_position();
        if target != current {
            stream
                .core
                .buffered_seek(target.wrapping_sub(current) as i32, SEEK_CUR);
        }
        let mut clamped = destination.len() as u32;
        if self.position_148 + clamped + extra > self.cached_size_150 {
            clamped = (self.cached_size_150).saturating_sub(self.position_148 + extra);
        }
        let got = stream.core.raw_read(&mut destination[..clamped as usize]);
        stream.position_148 = got as u32 + target;
        stream.position_shadow_14c = stream.position_148;
        got
    }

    /// IDA: 0042C4A0. Completion: 100%.
    fn buffered_archive_read(&mut self, destination: &mut [u8]) -> usize {
        let mut remaining = destination.len();
        let mut copied = 0;
        let available = (self.core.buffer_valid - self.core.buffer_position) as usize;
        if remaining > available {
            if available != 0 {
                let start = self.core.buffer_position as usize;
                let buffer = self
                    .core
                    .buffer
                    .as_ref()
                    .expect("a valid window has a buffer");
                destination[..available].copy_from_slice(&buffer[start..start + available]);
                remaining -= available;
                copied = available;
            }
            self.core.buffer_valid = 0;
            self.core.buffer_position = 0;
            if remaining > self.core.buffer_size as usize {
                return copied + self.read_at(&mut destination[copied..], copied as u32);
            }
            let loaded = match self.core.buffer.take() {
                Some(mut buffer) => {
                    let size = self.core.buffer_size as usize;
                    let loaded = self.read_at(&mut buffer[..size], copied as u32);
                    self.core.buffer = Some(buffer);
                    loaded
                }
                None => 0,
            };
            self.core.buffer_valid = loaded as u32;
            remaining = remaining.min(loaded);
        }
        if remaining != 0 {
            let start = self.core.buffer_position as usize;
            let buffer = self.core.buffer.as_ref().expect("bytes were loaded");
            destination[copied..copied + remaining]
                .copy_from_slice(&buffer[start..start + remaining]);
        }
        self.core.buffer_position += remaining as u32;
        remaining + copied
    }

    /// IDA: 0042C5A0 ReadString, through the stream at the record position.
    pub fn read_string(&mut self, out: &mut Vec<u8>) -> u32 {
        let mut stream = self.owner_154.stream.lock().expect("the archive lock");
        let target = self.position_148 + self.offset_158;
        if stream.effective_position() != target {
            stream.seek(target as i32, SEEK_SET);
        }
        let read = stream.read_string(out, self.cached_size_150);
        self.position_148 += read;
        read
    }
}

impl GameFile for ArchiveFile {
    /// IDA: N/A `ArchiveFile`'s side of `GameFile`.
    fn core(&self) -> &FileCore {
        &self.core
    }
    /// IDA: N/A `ArchiveFile`'s side of `GameFile`.
    fn core_mut(&mut self) -> &mut FileCore {
        &mut self.core
    }
    /// IDA: N/A `ArchiveFile`'s side of `GameFile`.
    fn proc_read(&mut self, buffer: &mut [u8]) -> usize {
        let got = self.buffered_archive_read(buffer);
        self.position_148 += got as u32;
        got
    }
    /// IDA: the read-only write slot.
    fn proc_write(&mut self, _source: &[u8]) -> usize {
        0
    }
    /// IDA: 0042C360: clamped to +0x150; a window overrun drops it.
    fn seek(&mut self, offset: i32, origin: i32) {
        let previous = self.position_148;
        self.position_148 = match origin {
            SEEK_CUR => (offset as u32)
                .wrapping_add(previous)
                .min(self.cached_size_150),
            SEEK_END => self.cached_size_150 - offset.unsigned_abs().min(self.cached_size_150),
            _ => (offset as u32).min(self.cached_size_150),
        };
        self.core.buffer_position = self
            .core
            .buffer_position
            .wrapping_add(self.position_148.wrapping_sub(previous));
        if self.core.buffer_position > self.core.buffer_valid {
            self.core.flush();
        }
    }
    /// IDA: N/A `ArchiveFile`'s side of `GameFile`.
    fn size(&mut self) -> u32 {
        self.cached_size_150
    }
}

impl Drop for ArchiveFile {
    /// IDA: N/A Rust's `Drop`.
    fn drop(&mut self) {
        self.owner_154.release();
    }
}

/// IDA: 0042D6D0 CompressedArchiveFile (0x174): +0x15C stream, +0x160 output, +0x164 uncompressed
/// size, +0x168 output size, +0x16C output position, +0x170 output valid. Completion: 100%.
pub struct CompressedArchiveFile {
    base: ArchiveFile,
    stream_15c: Option<Decompress>,
    output_160: Vec<u8>,
    uncompressed_size_164: u32,
    output_size_168: u32,
    output_position_16c: u32,
    output_valid_170: u32,
}

impl CompressedArchiveFile {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn new(
        path: &str,
        owner: Arc<ArchiveShared>,
        offset: u32,
        size: u32,
        options: i32,
    ) -> Self {
        let mut base = ArchiveFile::new(path, owner, offset, size, options);
        let limit = base.owner_154.buffer_limit.load(Ordering::Relaxed);
        let mut raw = [0; 4];
        base.position_148 += base.buffered_archive_read(&mut raw) as u32;
        let uncompressed_size_164 = u32::from_le_bytes(raw);
        let output_size_168 = uncompressed_size_164.min(limit);
        Self {
            base,
            stream_15c: Some(Decompress::new(true)),
            output_160: vec![0; (output_size_168 as usize).max(1)],
            uncompressed_size_164,
            output_size_168,
            output_position_16c: 0,
            output_valid_170: 0,
        }
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn stream_ready(&self) -> bool {
        self.stream_15c.is_some()
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn position(&self) -> u32 {
        self.base.position_148
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn uncompressed_size(&self) -> u32 {
        self.uncompressed_size_164
    }

    /// IDA: 0042C5F0: "This function has not been written yet."
    pub fn read_string(&mut self, _out: &mut Vec<u8>) -> u32 {
        emit(Level::Verbose, "This function has not been written yet.");
        0
    }

    /// IDA: see the words here, which name the addresses this relates to.
    /// One inflate pass into the output block, as 0042C6F0's inner loop runs. Returns bytes produced.
    fn inflate_block(&mut self) -> Option<u32> {
        let output_size = self.output_size_168 as usize;
        let mut produced = 0usize;
        if output_size == 0 {
            return Some(0);
        }
        loop {
            let input = (self.base.core.buffer_valid - self.base.core.buffer_position) as usize;
            let mut consumed = 0;
            if input != 0 {
                let start = self.base.core.buffer_position as usize;
                let buffer = self.base.core.buffer.as_ref()?;
                let stream = self.stream_15c.as_mut()?;
                let (in_before, out_before) = (stream.total_in(), stream.total_out());
                let status = stream.decompress(
                    &buffer[start..start + input],
                    &mut self.output_160[produced..output_size],
                    FlushDecompress::Sync,
                );
                consumed = (stream.total_in() - in_before) as usize;
                produced += (stream.total_out() - out_before) as usize;
                match status {
                    Err(_) => {
                        emit(
                            Level::Warning,
                            &format!(
                                "Error inflating ZLib stream for file {}.",
                                self.base.path_3c
                            ),
                        );
                        self.stream_15c = None;
                        return Some(produced as u32);
                    }
                    Ok(Status::StreamEnd) => return Some(produced as u32),
                    Ok(_) => {}
                }
            }
            self.base.core.buffer_position += consumed as u32;
            self.base.position_148 += consumed as u32;
            if consumed == input {
                self.base.core.flush();
                self.base.load_window();
                // Host guard: with no input left the original loops forever.
                if input == 0 && self.base.core.buffer_valid == 0 {
                    emit(
                        Level::Warning,
                        &format!(
                            "Host stopped an endless ZLib read for file {} (truncated compressed data).",
                            self.base.path_3c
                        ),
                    );
                    return Some(produced as u32);
                }
            }
            if produced == output_size {
                return Some(produced as u32);
            }
        }
    }

    /// IDA: 0042C6F0, with a null destination for 0042BB30's skip. Completion: 100%.
    fn read_inflated(&mut self, mut destination: Option<&mut [u8]>, size: usize) -> usize {
        let mut remaining = size;
        let available = (self.output_valid_170 - self.output_position_16c) as usize;
        let start = self.output_position_16c as usize;
        if remaining <= available {
            if let Some(destination) = destination {
                destination[..remaining]
                    .copy_from_slice(&self.output_160[start..start + remaining]);
            }
            self.output_position_16c += remaining as u32;
            return remaining;
        }
        let mut total = 0;
        if available != 0 {
            if let Some(destination) = destination.as_deref_mut() {
                destination[..available]
                    .copy_from_slice(&self.output_160[start..start + available]);
            }
            remaining -= available;
            total = available;
        }
        self.output_position_16c = 0;
        self.output_valid_170 = 0;
        if remaining == 0 || self.stream_15c.is_none() {
            return total;
        }
        loop {
            let Some(produced) = self.inflate_block() else {
                return total;
            };
            self.output_valid_170 = produced;
            if produced == 0 {
                return total;
            }
            let produced = produced as usize;
            if remaining <= produced {
                break;
            }
            if let Some(destination) = destination.as_deref_mut() {
                destination[total..total + produced].copy_from_slice(&self.output_160[..produced]);
            }
            total += produced;
            remaining -= produced;
        }
        if let Some(destination) = destination {
            destination[total..total + remaining].copy_from_slice(&self.output_160[..remaining]);
        }
        self.output_position_16c += remaining as u32;
        remaining + total
    }
}

impl GameFile for CompressedArchiveFile {
    /// IDA: N/A `CompressedArchiveFile`'s side of `GameFile`.
    fn core(&self) -> &FileCore {
        &self.base.core
    }
    /// IDA: N/A `CompressedArchiveFile`'s side of `GameFile`.
    fn core_mut(&mut self) -> &mut FileCore {
        &mut self.base.core
    }
    /// IDA: N/A `CompressedArchiveFile`'s side of `GameFile`.
    fn proc_read(&mut self, buffer: &mut [u8]) -> usize {
        let size = buffer.len();
        self.read_inflated(Some(buffer), size)
    }
    /// IDA: N/A `CompressedArchiveFile`'s side of `GameFile`.
    fn proc_write(&mut self, _source: &[u8]) -> usize {
        0
    }
    /// IDA: 0042BB30: any seek is a forward skip by `offset`.
    fn seek(&mut self, offset: i32, _origin: i32) {
        self.read_inflated(None, offset as u32 as usize);
    }
    /// IDA: N/A `CompressedArchiveFile`'s side of `GameFile`.
    fn size(&mut self) -> u32 {
        self.uncompressed_size_164
    }
}

/// IDA: the objects the open routes return, in place of their vtables.
pub enum OpenedFile {
    Ni(NiFile),
    Loose(BsFile),
    Packed(ArchiveFile),
    Compressed(CompressedArchiveFile),
}

impl OpenedFile {
    /// IDA: N/A the file behind its class, as a reader for 006F9980.
    #[must_use]
    pub fn into_game_file(self) -> Box<dyn GameFile> {
        match self {
            Self::Ni(file) => Box::new(file),
            Self::Loose(file) => Box::new(file),
            Self::Packed(file) => Box::new(file),
            Self::Compressed(file) => Box::new(file),
        }
    }

    // IDA: N/A a helper, with no address of its own.
    fn inner(&mut self) -> &mut dyn GameFile {
        match self {
            Self::Ni(file) => file,
            Self::Loose(file) => file,
            Self::Packed(file) => file,
            Self::Compressed(file) => file,
        }
    }

    /// IDA: N/A a helper, with no address of its own.
    pub fn read(&mut self, buffer: &mut [u8]) -> usize {
        self.inner().read(buffer)
    }
    /// IDA: the +0x04 read proc with its component list, which only a swapping file uses.
    pub fn read_components(&mut self, buffer: &mut [u8], components: Option<&[u32]>) -> usize {
        self.inner().read_components(buffer, components)
    }
    /// IDA: N/A a helper, with no address of its own.
    pub fn write(&mut self, source: &[u8]) -> usize {
        self.inner().write(source)
    }
    /// IDA: N/A a helper, with no address of its own.
    pub fn seek(&mut self, offset: i32, origin: i32) {
        self.inner().seek(offset, origin);
    }
    /// IDA: N/A a helper, with no address of its own.
    pub fn size(&mut self) -> u32 {
        self.inner().size()
    }
    /// IDA: N/A a helper, with no address of its own.
    pub fn read_line(&mut self, max: u32, delimiter: u16) -> (u32, Vec<u8>) {
        self.inner().read_line(max, delimiter)
    }
    /// IDA: N/A a helper, with no address of its own.
    pub fn good(&mut self) -> bool {
        self.inner().good()
    }
    /// IDA: N/A a helper, with no address of its own.
    pub fn buffer_size(&mut self) -> u32 {
        self.inner().buffer_size()
    }
    /// IDA: N/A a helper, with no address of its own.
    pub fn buffer_valid(&mut self) -> u32 {
        self.inner().buffer_valid()
    }
    /// IDA: N/A a helper, with no address of its own.
    pub fn has_handle(&mut self) -> bool {
        self.inner().has_handle()
    }
    /// IDA: BSFile +0x3C for the BSFile family; an NiFile has no path.
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::Ni(_) => None,
            Self::Loose(file) => Some(file.path()),
            Self::Packed(file) => Some(file.path()),
            Self::Compressed(file) => Some(file.base.path()),
        }
    }
    /// IDA: the `__RTDynamicCast` to BSFile that 00431690 applies: everything but NiFile.
    #[must_use]
    pub const fn is_bs_file(&self) -> bool {
        !matches!(self, Self::Ni(_))
    }
}
