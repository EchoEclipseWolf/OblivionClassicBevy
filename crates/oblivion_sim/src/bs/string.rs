//! BSStringT, the Bethesda string (pointer, length, capacity).

/// IDA: BSStringT {char* +0x00, u16 length +0x04, u16 capacity +0x06}.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BsStringT {
    pub data_00: Option<Vec<u8>>,
    pub length_04: u16,
    pub capacity_06: u16,
}

impl BsStringT {
    // IDA: 004028D0 Set(source, minimum): n = max(strlen, minimum) (minimum for a null source); n <= capacity reuses the buffer (n == 0 frees it and clears all
    // three fields); otherwise a new n + 1 block, capacity = min(n, 0xFFFF); length = min(n, 0xFFFF). Completion: 100%.
    pub fn set(&mut self, source: Option<&[u8]>, minimum_capacity: u32) -> bool {
        let mut size = minimum_capacity;
        if let Some(source) = source
            && source.len() as u32 > minimum_capacity
        {
            size = source.len() as u32;
        }
        if size <= u32::from(self.capacity_06) {
            if size == 0 {
                *self = Self::default();
                return false;
            }
        } else {
            self.capacity_06 = size.min(0xFFFF) as u16;
        }
        self.data_00 = Some(source.unwrap_or_default().to_vec());
        self.length_04 = size.min(0xFFFF) as u16;
        size != 0
    }
}
