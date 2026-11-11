//! NiFile: the buffered file under BSFile.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

pub const SEEK_CUR: i32 = 1;

pub const SEEK_END: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndianSwapStatus {
    Swapped,
    /// A zero component count, where the original loops forever.
    UnsupportedEndlessLoop,
}

/// IDA: 00748260 NiFile component swap: the list repeats over `data`; a lone one-byte component
/// returns at once. Completion: 100%.
pub fn ni_file_swap_components(data: &mut [u8], components: &[u32]) -> EndianSwapStatus {
    let mut covered = 0;
    while covered < data.len() {
        if components.is_empty() {
            return EndianSwapStatus::UnsupportedEndlessLoop;
        }
        for &component in components {
            let size = component as usize;
            match component {
                1 if components.len() == 1 => {
                    return EndianSwapStatus::Swapped;
                }
                2 | 4 | 8 => data[covered..covered + size].reverse(),
                _ => {}
            }
            covered += size;
        }
    }
    EndianSwapStatus::Swapped
}

/// IDA: NiFile's buffered core: +0x0C size, +0x10 valid, +0x14 position, +0x18 buffer, +0x1C file,
/// +0x20 mode, +0x24 good. Completion: 100%.
#[derive(Debug, Default)]
pub struct FileCore {
    pub(crate) working_directory: PathBuf,
    pub(crate) buffer_size: u32,
    pub(crate) buffer_valid: u32,
    pub(crate) buffer_position: u32,
    pub(crate) buffer: Option<Vec<u8>>,
    pub(crate) file: Option<File>,
    pub(crate) mode: u32,
    pub(crate) good: bool,
    pub(crate) swap: bool,
}

impl FileCore {
    /// IDA: +0x14 the buffer position.
    #[must_use]
    pub const fn buffer_position(&self) -> u32 {
        self.buffer_position
    }

    /// IDA: +0x20 the open mode.
    #[must_use]
    pub const fn mode(&self) -> u32 {
        self.mode
    }

    /// IDA: N/A whether a byte-swapped magic turned endian swapping on.
    #[must_use]
    pub const fn swap_active(&self) -> bool {
        self.swap
    }

    // IDA: N/A a helper, with no address of its own.
    pub(crate) fn allocate_buffer(&mut self, size: u32) {
        self.buffer = Some(vec![0; (size as usize).max(1)]);
    }

    /// IDA: N/A a helper, with no address of its own.
    /// `fread`: as much as the file gives.
    pub(crate) fn raw_read(&mut self, buffer: &mut [u8]) -> usize {
        let Some(file) = self.file.as_mut() else {
            return 0;
        };
        let mut total = 0;
        while total < buffer.len() {
            match file.read(&mut buffer[total..]) {
                Ok(0) | Err(_) => break,
                Ok(count) => total += count,
            }
        }
        total
    }

    /// IDA: 00747E90: read-only resets the window; otherwise pending bytes are written.
    pub(crate) fn flush(&mut self) -> bool {
        if self.mode == 0 {
            self.buffer_valid = 0;
            self.buffer_position = 0;
            return true;
        }
        let pending = self.buffer_position as usize;
        if pending != 0 {
            let written = match (self.file.as_mut(), self.buffer.as_ref()) {
                (Some(file), Some(buffer)) => file.write_all(&buffer[..pending]).is_ok(),
                _ => false,
            };
            if !written {
                self.good = false;
                return false;
            }
        }
        self.buffer_position = 0;
        true
    }

    /// IDA: 00747E20: a relative move inside the valid window stays there.
    pub(crate) fn buffered_seek(&mut self, offset: i32, origin: i32) {
        if !self.good {
            return;
        }
        let mut target = offset;
        if origin == SEEK_CUR {
            let moved = self.buffer_position as i32 + offset;
            if moved >= 0 && moved < self.buffer_valid as i32 {
                self.buffer_position = moved as u32;
                return;
            }
            target = self.buffer_position.wrapping_sub(self.buffer_valid) as i32 + offset;
        }
        self.flush();
        let from = match origin {
            SEEK_CUR => SeekFrom::Current(i64::from(target)),
            SEEK_END => SeekFrom::End(i64::from(target)),
            _ => SeekFrom::Start(target as u32 as u64),
        };
        self.good = self
            .file
            .as_mut()
            .is_some_and(|file| file.seek(from).is_ok());
    }

    /// IDA: 00747F00. Completion: 100%.
    pub(crate) fn buffered_read(&mut self, destination: &mut [u8]) -> usize {
        if !self.good {
            return 0;
        }
        let mut remaining = destination.len();
        let mut copied = 0;
        let available = (self.buffer_valid - self.buffer_position) as usize;
        if remaining > available {
            if available != 0 {
                let start = self.buffer_position as usize;
                let buffer = self.buffer.as_ref().expect("a valid window has a buffer");
                destination[..available].copy_from_slice(&buffer[start..start + available]);
                remaining -= available;
                copied = available;
            }
            self.flush();
            if remaining > self.buffer_size as usize {
                return copied + self.raw_read(&mut destination[copied..]);
            }
            let loaded = match self.buffer.take() {
                Some(mut buffer) => {
                    let size = self.buffer_size as usize;
                    let loaded = self.raw_read(&mut buffer[..size]);
                    self.buffer = Some(buffer);
                    loaded
                }
                None => 0,
            };
            self.buffer_valid = loaded as u32;
            remaining = remaining.min(loaded);
        }
        if remaining != 0 {
            let start = self.buffer_position as usize;
            let buffer = self.buffer.as_ref().expect("bytes were loaded");
            destination[copied..copied + remaining]
                .copy_from_slice(&buffer[start..start + remaining]);
        }
        self.buffer_position += remaining as u32;
        copied + remaining
    }

    /// IDA: 00747FC0. Completion: 100%.
    pub(crate) fn buffered_write(&mut self, source: &[u8]) -> usize {
        if !self.good {
            return 0;
        }
        let mut remaining = source.len();
        let mut copied = 0;
        let space = (self.buffer_size - self.buffer_position) as usize;
        if remaining > space {
            if space != 0 {
                let start = self.buffer_position as usize;
                let buffer = self.buffer.as_mut().expect("free space has a buffer");
                buffer[start..start + space].copy_from_slice(&source[..space]);
                remaining -= space;
                copied = space;
                self.buffer_position = self.buffer_size;
            }
            if !self.flush() {
                return 0;
            }
            if remaining >= self.buffer_size as usize {
                let written = self
                    .file
                    .as_mut()
                    .is_some_and(|file| file.write_all(&source[copied..]).is_ok());
                return copied + if written { remaining } else { 0 };
            }
        }
        if remaining != 0 {
            let start = self.buffer_position as usize;
            let buffer = self.buffer.as_mut().expect("the bytes fit a buffer");
            buffer[start..start + remaining].copy_from_slice(&source[copied..copied + remaining]);
        }
        self.buffer_position += remaining as u32;
        copied + remaining
    }

    // IDA: N/A a helper, with no address of its own.
    pub(crate) fn file_size_from_handle(&mut self) -> u32 {
        let Some(file) = self.file.as_mut() else {
            return 0;
        };
        let Ok(offset) = file.stream_position() else {
            return 0;
        };
        let end = file.seek(SeekFrom::End(0)).unwrap_or(0);
        let _ = file.seek(SeekFrom::Start(offset));
        end as u32
    }
}
