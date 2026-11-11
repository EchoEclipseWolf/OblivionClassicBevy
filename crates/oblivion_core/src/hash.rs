//! Original hashes. Bucket and lookup results depend on these bit for bit.

use crate::crt::{c_tolower, stricmp};

/// IDA: N/A FNV-1a over a dword's four bytes, as the evidence tools hash IDs. Not original code.
#[must_use]
pub fn fnv1a_dword(mut hash: u32, value: u32) -> u32 {
    for byte in value.to_le_bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// IDA: N/A the FNV-1a offset basis.
pub const FNV1A_BASIS: u32 = 0x811C_9DC5;

/// IDA: 008B0E80 slot hash: ((key >> 4) * 0x9E3779B1) & mask. Completion: 100%.
#[must_use]
pub const fn havok_pointer_hash(key: u32, mask: u32) -> u32 {
    (key >> 4).wrapping_mul(0x9E37_79B1) & mask
}

/// IDA: 007DAED0 NiTStringPointerMap_CalculateBucket: h = 33*h + signed byte, without case
/// folding. Callers take the modulo. Completion: 100%.
#[must_use]
pub fn ni_string_hash(key: &[u8]) -> u32 {
    key.iter().fold(0u32, |hash, &byte| {
        hash.wrapping_mul(33)
            .wrapping_add(i32::from(byte as i8) as u32)
    })
}

/// IDA: N/A a helper, with no address of its own.
/// A byte as the original reads it through a signed char, lowered.
fn lowered(byte: u8) -> i32 {
    c_tolower(i32::from(byte as i8))
}

/// IDA: 006FA1B0's rolling term: h = h * 0x1003F + lowered signed byte, over `count` bytes, reading
/// NUL past the end. Completion: 100%.
#[must_use]
pub fn bsa_rolling_hash(text: &[u8], count: u8) -> u32 {
    (0..usize::from(count)).fold(0u32, |hash, index| {
        hash.wrapping_mul(0x1003F)
            .wrapping_add(lowered(text.get(index).copied().unwrap_or(0)) as u32)
    })
}

// IDA: N/A a helper, with no address of its own.
fn c_string(text: &[u8]) -> &[u8] {
    text.iter()
        .position(|&byte| byte == 0)
        .map_or(text, |end| &text[..end])
}

/// IDA: 006FA1B0 / 006FA2D0 the BSA name hash. `extension` includes its dot; `None` is the folder
/// form. Verified against a shipped archive's folder and file records. Completion: 100%.
#[must_use]
pub fn bsa_name_hash(raw_name: &[u8], extension: Option<&[u8]>) -> u64 {
    let name = c_string(raw_name);
    let at = |index: isize| -> i32 {
        usize::try_from(index)
            .ok()
            .and_then(|index| name.get(index))
            .map_or(0, |&byte| lowered(byte))
    };
    let length = name.len() as u8;
    let mut bytes = [0u8; 4];
    bytes[2] = length;
    bytes[3] = at(0) as u8;
    bytes[0] = at(isize::from(length) - 1) as u8;
    bytes[1] = if length <= 2 {
        0
    } else {
        at(isize::from(length) - 2) as u8
    };
    let mut high = if length > 3 {
        bsa_rolling_hash(&name[1..], length - 3)
    } else {
        0
    };
    if let Some(extension) = extension {
        let extension = c_string(extension);
        high = high.wrapping_add(bsa_rolling_hash(extension, extension.len() as u8));
        // IDA: B25374 five 10-byte extension strings.
        const KNOWN: [&[u8]; 5] = [b"", b".NIF", b".KF", b".DDS", b".WAV"];
        if let Some(index) = KNOWN
            .iter()
            .position(|known| stricmp(known, extension) == 0)
        {
            let index = index as u32;
            bytes[3] = bytes[3].wrapping_add(((index & 0xFC) << 5) as u8);
            bytes[0] = bytes[0].wrapping_add(((index & 0xFE) << 6) as u8);
            bytes[1] = bytes[1].wrapping_add((index << 7) as u8);
        }
    }
    (u64::from(high) << 32) | u64::from(u32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    /// The shipped archive's own records are the oracle: every folder and file name in
    /// `Oblivion - Misc.bsa` must hash to the value stored beside it.
    #[test]
    fn bsa_hash_matches_every_record_of_a_shipped_archive() {
        let bytes = std::fs::read(fixtures::data_dir().join("Oblivion - Misc.bsa")).unwrap();
        let dword = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let qword = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        assert_eq!(&bytes[..4], b"BSA\0");
        assert_eq!(dword(4), 103);
        let (folders, files, names_size) = (dword(16), dword(20), dword(28));
        let mut cursor = 36 + 16 * folders;
        let names_at = 36 + 16 * folders + dword(24) + folders + 16 * files;
        let mut names = bytes[names_at..names_at + names_size].split(|&byte| byte == 0);
        let mut checked = 0;
        for folder in 0..folders {
            let record = 36 + 16 * folder;
            let count = dword(record + 8);
            let name_length = usize::from(bytes[cursor]);
            let name = &bytes[cursor + 1..cursor + name_length];
            assert_eq!(bsa_name_hash(name, None), qword(record), "folder {name:?}");
            checked += 1;
            cursor += 1 + name_length;
            for _ in 0..count {
                let file_name = names.next().unwrap();
                let dot = file_name.iter().rposition(|&byte| byte == b'.').unwrap();
                let hash = bsa_name_hash(&file_name[..dot], Some(&file_name[dot..]));
                assert_eq!(
                    hash,
                    qword(cursor),
                    "file {:?}",
                    String::from_utf8_lossy(file_name)
                );
                checked += 1;
                cursor += 16;
            }
        }
        assert_eq!(checked, folders + files);
    }

    #[test]
    fn small_hashes_match_their_definitions() {
        assert_eq!(havok_pointer_hash(0x10, 0xFF), 0x9E37_79B1 & 0xFF);
        assert_eq!(ni_string_hash(b"ab"), 33 * 97 + 98);
        // A high byte is sign-extended before it is added.
        assert_eq!(ni_string_hash(&[0xFF]), u32::MAX);
        // The evidence tools' value for the dword 0x35 alone.
        assert_eq!(fnv1a_dword(FNV1A_BASIS, 0x35), {
            let mut hash = FNV1A_BASIS;
            for byte in [0x35u32, 0, 0, 0] {
                hash = (hash ^ byte).wrapping_mul(0x0100_0193);
            }
            hash
        });
    }
}
