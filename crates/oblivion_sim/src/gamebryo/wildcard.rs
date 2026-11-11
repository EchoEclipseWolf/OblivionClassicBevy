//! 006FA6F0 Wildcard_MatchNormalized and 006FA410 Wildcard_Match, Gamebryo's case-folding wildcard match (the addresses sit
//! among the NiObjectNET functions).

use oblivion_core::crt::c_toupper;
use oblivion_core::error::GameError;

/// IDA: 006FA600 with no 00B3F480 bits, the only value it has: `00B3F480` has no direct writer in the exe, so it stays 0. Bit 3 skips leading spaces, bits 1 and 2 run _splitpath and _makepath; they stop here. The copy goes into 0x104 stack bytes of 006FA6F0's frame, with no length check.
const WILDCARD_BUFFER: usize = 0x104;

// IDA: 006FA3F0, bit 0 of 00B3F480 clear: `toupper` of the sign-extended byte, kept as a byte.
fn wildcard_fold(byte: u8) -> u8 {
    c_toupper(i32::from(byte as i8)) as u8
}

// IDA: N/A a helper, with no address of its own.
fn wildcard_byte(text: &[u8], index: usize) -> u8 {
    text.get(index).copied().unwrap_or(0)
}

// IDA: 006FA410(text, pattern), recursive. Pattern end: 1 when the text ends too, else 0. '?' takes a byte with no end test (reading past the text is a stop). '*': 1 at the pattern's end, 2 for an empty text; else each suffix in turn, returning the first nonzero, then 2. '[': 0 on an empty text; '^' negates; the closing ']' after '\\' escapes (none: 0). Inside, a leading '-' reads literally, '\\' escapes the next, '-' takes the previous byte as the low end, and a byte followed by '-' is skipped as a range start. A range is walked with signed bytes (006FA53C `jg`, 006FA562 `jle`), comparing the zero-extended fold of each with the sign-extended fold of the text byte (006FA53E `movsx`, 006FA556 `movzx`), so a text byte above 0x7F never matches. A hit: 0 when negated, else the rest. No hit: the rest when negated, else 0. '\\': the next byte as a literal, 0 at the pattern's end. A literal compares the two folds, and goes on with the rest. Completion: 100% with 00B3F480 = 0.
fn wildcard_body(text: &[u8], at: usize, pattern: &[u8], from: usize) -> Result<i32, GameError> {
    let mut p = from;
    let mut c = wildcard_byte(pattern, p);
    p += 1;
    if c == 0 {
        return Ok(i32::from(wildcard_byte(text, at) == 0));
    }
    if c == b'?' {
        if at >= text.len() {
            return Err(GameError::unclassified(
                0x006FA43B,
                "006FA43B: '?' past the end of the text",
            ));
        }
        return wildcard_body(text, at + 1, pattern, p);
    }
    if c == b'*' {
        if wildcard_byte(pattern, p) == 0 {
            return Ok(1);
        }
        let mut start = at;
        while wildcard_byte(text, start) != 0 {
            let result = wildcard_body(text, start, pattern, p)?;
            if result != 0 {
                return Ok(result);
            }
            start += 1;
        }
        return Ok(2);
    }
    if c == b'[' {
        if wildcard_byte(text, at) == 0 {
            return Ok(0);
        }
        let negate = wildcard_byte(pattern, p) == b'^';
        if negate {
            p += 1;
        }
        let first = wildcard_byte(pattern, p);
        let mut end = p;
        if first != 0 {
            let mut escaped = false;
            loop {
                if escaped {
                    escaped = false;
                } else {
                    let byte = wildcard_byte(pattern, end);
                    if byte == b'\\' {
                        escaped = true;
                    } else if byte == b']' {
                        break;
                    }
                }
                end += 1;
                if wildcard_byte(pattern, end) == 0 {
                    break;
                }
            }
        }
        if wildcard_byte(pattern, end) != b']' {
            return Ok(0);
        }
        let mut literal = first == b'-';
        let mut low = 0_i8;
        let wanted = i32::from(wildcard_fold(wildcard_byte(text, at)) as i8);
        while p < end {
            if !literal {
                let byte = wildcard_byte(pattern, p);
                if byte == b'\\' {
                    literal = true;
                    p += 1;
                    continue;
                }
                if byte == b'-' {
                    low = wildcard_byte(pattern, p - 1) as i8;
                    p += 1;
                    continue;
                }
            }
            let high = wildcard_byte(pattern, p) as i8;
            if wildcard_byte(pattern, p + 1) != b'-' {
                let mut value = if low == 0 { high } else { low };
                while value <= high {
                    if i32::from(wildcard_fold(value as u8)) == wanted {
                        if negate {
                            return Ok(0);
                        }
                        return wildcard_body(text, at + 1, pattern, end + 1);
                    }
                    if value == i8::MAX {
                        return Err(GameError::unclassified(
                            0x006FA55D,
                            "006FA55D: a range that ends at 0x7F never ends",
                        ));
                    }
                    value += 1;
                }
            }
            literal = false;
            low = 0;
            p += 1;
        }
        if negate {
            return wildcard_body(text, at + 1, pattern, end + 1);
        }
        return Ok(0);
    }
    if c == b'\\' {
        c = wildcard_byte(pattern, p);
        p += 1;
        if c == 0 {
            return Ok(0);
        }
    }
    if wildcard_fold(wildcard_byte(text, at)) != wildcard_fold(c) {
        return Ok(0);
    }
    wildcard_body(text, at + 1, pattern, p)
}

/// IDA: 006FA6F0 Wildcard_Match(text, pattern): 006FA600 copies both, and only 006FA410's result 1 answers true (`sub eax,1; neg; sbb; add eax,1`: its 2 answers false). Completion: 100% with 00B3F480 = 0.
/// # Errors
/// A text past the stack buffer, or a stop in 006FA410.
pub fn wildcard_match(text: &[u8], pattern: &[u8]) -> Result<bool, GameError> {
    if text.len() >= WILDCARD_BUFFER || pattern.len() >= WILDCARD_BUFFER {
        return Err(GameError::unclassified(
            0x006FA71C,
            "006FA71C: a text past the 0x104 stack bytes",
        ));
    }
    Ok(wildcard_body(text, 0, pattern, 0)? == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 006FA410's arms, and 006FA6F0's "only 1 is true".
    #[test]
    fn the_wildcard_keeps_its_arms() {
        let test = |text: &[u8], pattern: &[u8]| wildcard_body(text, 0, pattern, 0).unwrap();
        assert_eq!(test(b"", b""), 1);
        assert_eq!(test(b"a", b""), 0);
        assert_eq!(test(b"abc", b"a?c"), 1);
        assert!(wildcard_body(b"a", 0, b"a?", 0).is_err(), "006FA43B");
        assert_eq!(test(b"abc", b"*"), 1);
        assert_eq!(test(b"", b"*a"), 2);
        assert_eq!(test(b"abc", b"*c"), 1);
        assert_eq!(test(b"abc", b"*x"), 2);
        assert_eq!(test(b"bip01", b"BIP[0-9][0-9]"), 1, "the fold");
        assert_eq!(test(b"Bip0a", b"Bip[0-9][0-9]"), 0);
        assert_eq!(
            test(b"Bip1", b"Bip[0-9][0-9]"),
            0,
            "an empty text fails a class"
        );
        assert_eq!(test(b"x", b"[^abc]"), 1);
        assert_eq!(test(b"a", b"[^abc]"), 0);
        assert_eq!(test(b"-", b"[-a]"), 1, "a leading '-' is literal");
        assert_eq!(test(b"]", b"[\\]]"), 1, "an escaped ']'");
        assert_eq!(test(b"a", b"[a"), 0, "no closing ']'");
        assert_eq!(test(b"*", b"\\*"), 1);
        assert_eq!(test(b"a", b"\\"), 0, "'\\' at the end");
        // 006FA53E `movsx` against 006FA556 `movzx`: a text byte above 0x7F never matches a class, though a literal compare takes it.
        assert_eq!(test(b"\xC8", b"[\xC8]"), 0);
        assert_eq!(test(b"\xC8", b"\xC8"), 1);
        assert!(!wildcard_match(b"", b"*a").unwrap(), "2 isn't true");
        assert!(wildcard_match(b"Bip01", b"Bip[0-9][0-9]").unwrap());
        assert!(wildcard_match(&[b'a'; 0x104], b"*").is_err());
    }
}
