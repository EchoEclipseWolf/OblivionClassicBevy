//! `GetPrivateProfile*` on Linux. The original and the C++ port call the Win32 functions; there is
//! no OS implementation here, so their INI rules are ported as a recorded host substitution.
//!
//! IDA: N/A. The rules follow Wine's documented behaviour for these APIs, which its test suite
//! checks against Windows: the first matching section only, the first matching key, whitespace
//! trimmed, one matched pair of quotes removed, output truncated to size - 1.
//! Completion: 90%.
//! TODO: [ ] Compare against a real Windows run for duplicate sections and for `0x` prefixes in
//! GetPrivateProfileInt. Shipped INI values use neither.

use std::path::Path;

use crate::crt::stricmp;

// IDA: N/A a helper, with no address of its own.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

// IDA: N/A a helper, with no address of its own.
fn trim(mut text: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = text {
        if !is_space(*first) {
            break;
        }
        text = rest;
    }
    while let [rest @ .., last] = text {
        if !is_space(*last) {
            break;
        }
        text = rest;
    }
    text
}

/// IDA: N/A a helper, with no address of its own.
/// The raw value of the first matching key in the first matching section.
fn find_value<'a>(file: &'a [u8], section: &[u8], key: &[u8]) -> Option<&'a [u8]> {
    let mut in_section = section.is_empty();
    let mut matched_before = false;
    for line in file.split(|&byte| byte == b'\n') {
        let line = trim(line);
        if let [b'[', rest @ ..] = line
            && let Some(end) = rest.iter().rposition(|&byte| byte == b']')
        {
            if matched_before {
                return None;
            }
            in_section = stricmp(&rest[..end], section) == 0;
            matched_before = in_section;
            continue;
        }
        if !in_section || line.is_empty() || line[0] == b';' {
            continue;
        }
        let (name, value) = match line.iter().position(|&byte| byte == b'=') {
            Some(at) => (trim(&line[..at]), trim(&line[at + 1..])),
            None => (line, &line[..0]),
        };
        if stricmp(name, key) == 0 {
            return Some(value);
        }
    }
    None
}

/// IDA: N/A a helper, with no address of its own.
/// `GetPrivateProfileString(section, key, default, buffer, size, file)`: the returned bytes are
/// what the buffer holds without its NUL, so their length is the API's return value. A missing
/// file or key gives the default, with its trailing spaces removed.
#[must_use]
pub fn read_string(
    file: &Path,
    section: &[u8],
    key: &[u8],
    default: Option<&[u8]>,
    size: usize,
) -> Vec<u8> {
    let bytes = std::fs::read(file).unwrap_or_default();
    let mut value: &[u8] = match find_value(&bytes, section, key) {
        Some(value) => {
            let mut value = value;
            if let [quote @ (b'\'' | b'"'), rest @ ..] = value
                && let [inner @ .., last] = rest
                && last == quote
            {
                value = inner;
            }
            value
        }
        None => {
            let mut value = default.unwrap_or(b"");
            while let [rest @ .., b' '] = value {
                value = rest;
            }
            value
        }
    };
    if let Some(end) = value.iter().position(|&byte| byte == 0) {
        value = &value[..end];
    }
    value[..value.len().min(size.saturating_sub(1))].to_vec()
}

/// IDA: N/A a helper, with no address of its own.
/// `GetPrivateProfileInt`: the default when the value is empty or missing; otherwise a signed
/// parse that accepts `0x`, `0o` and `0b` prefixes, stops at the first bad digit and wraps.
#[must_use]
pub fn read_int(file: &Path, section: &[u8], key: &[u8], default: i32) -> u32 {
    let text = read_string(file, section, key, Some(b""), 30);
    if text.is_empty() {
        return default as u32;
    }
    let mut at = 0;
    while text.get(at).copied().is_some_and(is_space) {
        at += 1;
    }
    let negative = text.get(at) == Some(&b'-');
    if matches!(text.get(at), Some(b'-' | b'+')) {
        at += 1;
    }
    let mut radix = 10;
    if text.get(at) == Some(&b'0') {
        let prefix = match text.get(at + 1) {
            Some(b'x' | b'X') => 16,
            Some(b'o' | b'O') => 8,
            Some(b'b' | b'B') => 2,
            _ => 10,
        };
        if prefix != 10 {
            radix = prefix;
            at += 2;
        }
    }
    let mut value = 0u32;
    while let Some(digit) = text
        .get(at)
        .and_then(|&byte| char::from(byte).to_digit(radix))
    {
        value = value.wrapping_mul(radix).wrapping_add(digit);
        at += 1;
    }
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_ini(name: &str, text: &str, check: impl FnOnce(&Path)) {
        let path =
            std::env::temp_dir().join(format!("recomp_profile_{}_{name}.ini", std::process::id()));
        std::fs::write(&path, text).unwrap();
        check(&path);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn strings_follow_the_win32_rules() {
        let text = "before=1\r\n[Sec]\r\n  Key = spaced value  \r\nquoted=\"a b\"\r\none='x\r\n;cmt=9\r\n\
                    empty=\r\nbare\r\nkey=second\r\n[Other]\r\nonly=1\r\n[SEC]\r\nlater=2\r\n";
        with_ini("strings", text, |ini| {
            let read =
                |section: &[u8], key: &[u8]| read_string(ini, section, key, Some(b"dflt  "), 0x100);
            assert_eq!(
                read(b"sec", b"KEY"),
                b"spaced value",
                "case and trimming; the first key wins"
            );
            assert_eq!(read(b"Sec", b"quoted"), b"a b");
            assert_eq!(read(b"Sec", b"one"), b"'x", "an unmatched quote stays");
            assert_eq!(read(b"Sec", b"empty"), b"");
            assert_eq!(read(b"Sec", b"bare"), b"");
            assert_eq!(read(b"Sec", b";cmt"), b"dflt", "a comment is not a key");
            assert_eq!(
                read(b"Sec", b"missing"),
                b"dflt",
                "default without trailing spaces"
            );
            assert_eq!(
                read(b"Sec", b"later"),
                b"dflt",
                "only the first matching section"
            );
            assert_eq!(read(b"Nope", b"key"), b"dflt");
            assert_eq!(
                read_string(ini, b"Sec", b"Key", None, 4),
                b"spa",
                "truncated to size - 1"
            );
            assert_eq!(read_string(ini, b"Sec", b"none", None, 8), b"");
        });
    }

    #[test]
    fn ints_parse_as_the_win32_call_does() {
        with_ini(
            "ints",
            "[S]\na=-7\nb=12x\nc=\nd=0x10\ne=abc\nf=  5\n",
            |ini| {
                assert_eq!(read_int(ini, b"S", b"a", 1), (-7i32) as u32);
                assert_eq!(read_int(ini, b"S", b"b", 1), 12);
                assert_eq!(read_int(ini, b"S", b"c", 3), 3);
                assert_eq!(read_int(ini, b"S", b"d", 1), 16);
                assert_eq!(read_int(ini, b"S", b"e", 1), 0);
                assert_eq!(read_int(ini, b"S", b"f", 1), 5);
                assert_eq!(read_int(ini, b"S", b"missing", -1), u32::MAX);
            },
        );
        assert_eq!(read_int(Path::new("/nonexistent/x.ini"), b"S", b"k", 9), 9);
    }

    /// The pinned INI the evidence runs used reads through these rules.
    #[test]
    fn the_pinned_ini_reads() {
        let ini = crate::fixtures::root().join("config/Oblivion.ini");
        assert!(ini.exists());
        assert_eq!(
            read_string(&ini, b"General", b"sTestFile1", Some(b"x"), 0x100),
            b""
        );
    }
}
