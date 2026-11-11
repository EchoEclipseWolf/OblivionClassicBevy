//! IDA: N/A a bounded little-endian cursor over a block's bytes, standing in for NiStream's read
//! callbacks. A short read is an error here; the original's short-read side effects are not modelled.

use crate::error::GameError;

#[derive(Clone, Copy, Debug)]
pub struct ByteReader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> ByteReader<'a> {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.at
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.bytes.len() - self.at
    }

    /// IDA: N/A a helper, with no address of its own.
    pub fn take(&mut self, count: usize, error: &'static str) -> Result<&'a [u8], GameError> {
        if count > self.remaining() {
            return Err(GameError::stream(error));
        }
        let slice = &self.bytes[self.at..self.at + count];
        self.at += count;
        Ok(slice)
    }

    /// IDA: N/A a helper, with no address of its own.
    pub fn u8(&mut self, error: &'static str) -> Result<u8, GameError> {
        Ok(self.take(1, error)?[0])
    }

    /// IDA: N/A a helper, with no address of its own.
    pub fn u16(&mut self, error: &'static str) -> Result<u16, GameError> {
        let bytes = self.take(2, error)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// IDA: N/A a helper, with no address of its own.
    pub fn u32(&mut self, error: &'static str) -> Result<u32, GameError> {
        let bytes = self.take(4, error)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// IDA: N/A a helper, with no address of its own.
    /// The raw bits as an f32; the bits are preserved.
    pub fn f32(&mut self, error: &'static str) -> Result<f32, GameError> {
        Ok(f32::from_bits(self.u32(error)?))
    }
}
