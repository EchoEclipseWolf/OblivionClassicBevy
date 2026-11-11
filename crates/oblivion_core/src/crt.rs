//! The MSVC CRT behaviour the original links statically.

use std::io::Read;

/// IDA: 0047DF98 `_rand` / `srand`: the CRT's per-thread LCG. The C++ port called the host CRT, so
/// this is new here; the sequence is verified against the 157 values of P14-RUN-004.
/// Completion: 100%.
#[derive(Clone, Copy, Debug)]
pub struct MsvcRand {
    holdrand: u32,
}

impl Default for MsvcRand {
    /// IDA: N/A the CRT's unseeded state is `srand(1)`.
    fn default() -> Self {
        Self { holdrand: 1 }
    }
}

impl MsvcRand {
    /// IDA: `srand(seed)`.
    pub fn srand(&mut self, seed: u32) {
        self.holdrand = seed;
    }

    /// IDA: N/A `_getptd()+0x14`, for a test that replays a recorded stream.
    #[must_use]
    pub const fn holdrand(&self) -> u32 {
        self.holdrand
    }

    /// IDA: `_rand`: holdrand * 214013 + 2531011, bits 16..30.
    pub fn rand(&mut self) -> i32 {
        self.holdrand = self.holdrand.wrapping_mul(214013).wrapping_add(2531011);
        ((self.holdrand >> 16) & 0x7FFF) as i32
    }
}

/// IDA: 0047DF80 Random_Next's state: 00B069C3, the first-use flag (image byte 1).
#[derive(Clone, Copy, Debug)]
pub struct RandomGlobals {
    pub first_use_b069c3: u8,
}

impl Default for RandomGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            first_use_b069c3: 1,
        }
    }
}

/// IDA: N/A the host seam of 0047DF80 Random_Next: 0047DFA8 _time64(0), 0047DF89/0047DFAE _srand and 0047DF98/0047DFBD _rand. A test injects the run's draw, since P11-RUN-001's _time64 is recorded later than the seed it fits.
pub trait RandomSource {
    /// IDA: 0047DFA8 _time64(0).
    fn time64(&mut self) -> i64;
    /// IDA: _srand.
    fn srand(&mut self, seed: u32);
    /// IDA: _rand.
    fn rand(&mut self) -> i32;
}

// IDA: 0047DF80 Random_Next(seed) (__cdecl): a nonzero seed: srand(seed), 00B069C3 = 0; otherwise, while 00B069C3 is set, srand(_time64(0)) and 00B069C3 = 0; then rand(). `time64` is read only
// on that first use, as in the original. The CRT is a seam, because a test injects its result. Completion: 100%.
pub fn random_next<S: RandomSource + ?Sized>(
    seed: u32,
    globals: &mut RandomGlobals,
    source: &mut S,
) -> i32 {
    if seed != 0 {
        source.srand(seed);
        globals.first_use_b069c3 = 0;
    } else if globals.first_use_b069c3 != 0 {
        let time = source.time64();
        source.srand(time as u32);
        globals.first_use_b069c3 = 0;
    }
    source.rand()
}

/// IDA: N/A a C string's bytes, as `strlen` bounds them: everything before the first NUL (all of them with none).
#[must_use]
pub fn c_prefix(text: &[u8]) -> &[u8] {
    text.split(|&byte| byte == 0).next().unwrap_or(text)
}

/// IDA: CRT `tolower` in the "C" locale (`__locale_changed` 0, P10-RUN-001).
#[must_use]
pub const fn c_tolower(c: i32) -> i32 {
    if c >= b'A' as i32 && c <= b'Z' as i32 {
        c + 0x20
    } else {
        c
    }
}

/// IDA: CRT `toupper` 0098664D in the "C" locale (`__locale_changed` 0, P10-RUN-001).
#[must_use]
pub const fn c_toupper(c: i32) -> i32 {
    if c >= b'a' as i32 && c <= b'z' as i32 {
        c - 0x20
    } else {
        c
    }
}

/// IDA: `_stricmp` in the "C" locale: stops at the first NUL or difference, returns the lowered
/// byte difference. Bytes are unsigned. A slice's end reads as NUL.
/// Completion: 100%.
#[must_use]
pub fn stricmp(left: &[u8], right: &[u8]) -> i32 {
    strnicmp(left, right, usize::MAX)
}

/// IDA: `_strnicmp` in the "C" locale. Completion: 100%.
#[must_use]
pub fn strnicmp(left: &[u8], right: &[u8], count: usize) -> i32 {
    for index in 0..count {
        let f = c_tolower(i32::from(left.get(index).copied().unwrap_or(0)));
        let l = c_tolower(i32::from(right.get(index).copied().unwrap_or(0)));
        if f == 0 || f != l {
            return f - l;
        }
    }
    0
}

/// IDA: CRT `atol`: optional whitespace and sign, then digits.
/// Completion: 90%.
/// TODO: [ ] The C++ port used the host UCRT's `atol`, which clamps on overflow. The original's
/// static `atol` at 009A253C may wrap instead; check it before any path can pass a huge value.
#[must_use]
pub fn atol(text: &[u8]) -> i32 {
    let mut index = 0;
    while matches!(
        text.get(index),
        Some(b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
    ) {
        index += 1;
    }
    let negative = text.get(index) == Some(&b'-');
    if matches!(text.get(index), Some(b'-' | b'+')) {
        index += 1;
    }
    let mut value: i64 = 0;
    while let Some(digit @ b'0'..=b'9') = text.get(index) {
        value = (value * 10 + i64::from(digit - b'0')).min(1 << 32);
        index += 1;
    }
    let value = if negative { -value } else { value };
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// IDA: 00982B96 CRT `strtol(text, &end, 16)`: optional whitespace and sign, an optional `0x`, then hex digits up to the
/// first bad one. Overflow clamps to LONG_MAX or LONG_MIN, as the CRT does; no digits is 0. Completion: 100% of the
/// return value; `end` is not used by the one caller, 00404540.
#[must_use]
pub fn strtol_hex(text: &[u8]) -> i32 {
    let mut index = 0;
    while matches!(
        text.get(index),
        Some(b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
    ) {
        index += 1;
    }
    let negative = text.get(index) == Some(&b'-');
    if matches!(text.get(index), Some(b'-' | b'+')) {
        index += 1;
    }
    if text.get(index) == Some(&b'0') && matches!(text.get(index + 1), Some(b'x' | b'X')) {
        index += 2;
    }
    let mut value: u64 = 0;
    while let Some(digit) = text
        .get(index)
        .and_then(|&byte| char::from(byte).to_digit(16))
    {
        value = (value * 16 + u64::from(digit)).min(1 << 32);
        index += 1;
    }
    if negative {
        if value > 0x8000_0000 {
            i32::MIN
        } else {
            (value as i64).wrapping_neg() as i32
        }
    } else if value > 0x7FFF_FFFF {
        i32::MAX
    } else {
        value as i32
    }
}

/// IDA: the sscanf `%f` conversion: the longest valid float prefix after whitespace, rounded
/// correctly to f32. `None` is a failed conversion (the original leaves the destination alone).
/// Completion: 90%.
/// TODO: [ ] `inf` and `nan` spellings are not accepted here, and a dangling exponent (`1e`) stops
/// before the `e`, where the CRT's `sscanf` may fail the conversion. Compare the original CRT.
#[must_use]
pub fn scan_f32(text: &[u8]) -> Option<(f32, usize)> {
    let mut index = 0;
    while matches!(
        text.get(index),
        Some(b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
    ) {
        index += 1;
    }
    let start = index;
    if matches!(text.get(index), Some(b'-' | b'+')) {
        index += 1;
    }
    let digits_from = index;
    while text.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    let mut digits = index - digits_from;
    if text.get(index) == Some(&b'.') {
        index += 1;
        let fraction_from = index;
        while text.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        digits += index - fraction_from;
    }
    if digits == 0 {
        return None;
    }
    if matches!(text.get(index), Some(b'e' | b'E')) {
        let mut exponent = index + 1;
        if matches!(text.get(exponent), Some(b'-' | b'+')) {
            exponent += 1;
        }
        if text.get(exponent).is_some_and(u8::is_ascii_digit) {
            while text.get(exponent).is_some_and(u8::is_ascii_digit) {
                exponent += 1;
            }
            index = exponent;
        }
    }
    let number = std::str::from_utf8(&text[start..index]).ok()?;
    Some((number.parse().ok()?, index))
}

/// IDA: sscanf "%f, %f" (0053A1B0 and its siblings): the fields converted, stopping at the first
/// mismatch. The comma is literal; the format's space allows any whitespace. An unconverted field
/// keeps the caller's value. Completion: 90% (the `scan_f32` limits apply).
pub fn scan_f32_pair(text: &[u8], first: &mut f32, second: &mut f32) -> i32 {
    let Some((value, used)) = scan_f32(text) else {
        return 0;
    };
    *first = value;
    if text.get(used) != Some(&b',') {
        return 1;
    }
    let Some((value, _)) = scan_f32(&text[used + 1..]) else {
        return 1;
    };
    *second = value;
    2
}

/// IDA: sscanf "%u,%u,.." (004A8979, 004A89F6): up to `out.len()` comma-separated values, returning
/// the count converted. Unconverted entries keep their value. `%u` accepts a sign and wraps.
/// Completion: 90%.
/// TODO: [ ] Overflow past 32 bits wraps here; compare the original CRT.
pub fn scan_u32_list(text: &[u8], out: &mut [u32]) -> i32 {
    let mut at = 0;
    let mut fields = 0;
    for (index, slot) in out.iter_mut().enumerate() {
        if index > 0 {
            if text.get(at) != Some(&b',') {
                break;
            }
            at += 1;
        }
        while matches!(
            text.get(at),
            Some(b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
        ) {
            at += 1;
        }
        let negative = text.get(at) == Some(&b'-');
        if matches!(text.get(at), Some(b'-' | b'+')) {
            at += 1;
        }
        let start = at;
        let mut value = 0u32;
        while let Some(digit @ b'0'..=b'9') = text.get(at) {
            value = value.wrapping_mul(10).wrapping_add(u32::from(digit - b'0'));
            at += 1;
        }
        if at == start {
            break;
        }
        *slot = if negative {
            value.wrapping_neg()
        } else {
            value
        };
        fields += 1;
    }
    fields
}

/// IDA: N/A CRT `_gcvt(value, digits, buffer)`: `digits` significant digits, in fixed form when the decimal exponent is
/// at least -5 and below `digits`, else in the CRT's `d.ddde+XXX` form with a three-digit exponent. Trailing zeros go;
/// an integral value keeps its point, as `_gcvt(1.0, 12)` gives "1." (documented MSVC behaviour).
/// Completion: 90% (no original run recorded a converted string).
/// TODO: [ ] Compare with an original run: the MSVC 2005-era CRT's text for 0.0 and small magnitudes.
#[must_use]
pub fn gcvt(value: f64, digits: usize) -> String {
    if value == 0.0 {
        return "0.".to_owned();
    }
    let scientific = format!("{:.*e}", digits - 1, value);
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("`e` formatting has an exponent");
    let exponent: i32 = exponent
        .parse()
        .expect("`e` formatting's exponent is a number");
    if exponent < -5 || exponent >= digits as i32 {
        let mantissa = mantissa.trim_end_matches('0');
        let sign = if exponent < 0 { '-' } else { '+' };
        return format!("{mantissa}e{sign}{:03}", exponent.abs());
    }
    let fixed = format!(
        "{:.*}",
        (digits as i32 - 1 - exponent).max(0) as usize,
        value
    );
    if fixed.contains('.') {
        fixed.trim_end_matches('0').to_owned()
    } else {
        fixed + "."
    }
}

/// IDA: 004A8903 sprintf "%f": six decimals, exact decimal expansion (Rust and the UCRT both
/// round the exact binary value). Completion: 90%.
/// TODO: [ ] Non-finite spellings (`-nan(ind)`, `inf`) are not reproduced.
#[must_use]
pub fn format_f(value: f64) -> String {
    format!("{value:.6}")
}

/// IDA: CRT text-mode `fopen(.., "r")` plus `fgets`: CRLF reads as LF, a lone CR is kept, Ctrl-Z
/// ends the file, and `fgets(buffer, size)` returns at most size - 1 bytes through the first LF.
/// Completion: 100% for the `fgets` paths the port reaches (00404B96 size 0x200, 0042.. size 260).
#[derive(Debug)]
pub struct TextModeFile {
    bytes: Vec<u8>,
    position: usize,
    hit_eof: bool,
    eof_flag: bool,
}

impl TextModeFile {
    /// IDA: N/A a helper, with no address of its own.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            position: 0,
            hit_eof: false,
            eof_flag: false,
        }
    }

    /// IDA: N/A a helper, with no address of its own.
    pub fn from_reader(mut reader: impl Read) -> std::io::Result<Self> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        Ok(Self::from_bytes(bytes))
    }

    // IDA: N/A a helper, with no address of its own.
    fn next_byte(&mut self) -> Option<u8> {
        if self.hit_eof {
            self.eof_flag = true;
            return None;
        }
        let Some(&byte) = self.bytes.get(self.position) else {
            self.eof_flag = true;
            return None;
        };
        if byte == 0x1A {
            self.hit_eof = true;
            self.eof_flag = true;
            return None;
        }
        self.position += 1;
        if byte == b'\r' && self.bytes.get(self.position) == Some(&b'\n') {
            self.position += 1;
            return Some(b'\n');
        }
        Some(byte)
    }

    /// IDA: N/A an accessor.
    /// `feof`: set only once a read has run into the end.
    #[must_use]
    pub const fn feof(&self) -> bool {
        self.eof_flag
    }

    /// IDA: N/A a helper, with no address of its own.
    /// `fgets(buffer, size)`. `None` when nothing was read, like a null return.
    pub fn fgets(&mut self, size: usize) -> Option<Vec<u8>> {
        let mut line = Vec::new();
        while line.len() + 1 < size {
            let Some(byte) = self.next_byte() else { break };
            line.push(byte);
            if byte == b'\n' {
                break;
            }
        }
        (!line.is_empty()).then_some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IDA: 0040463D: `R 108 404642 eax=11FFFF` for the INI's `Forward=0011FFFF` (P04-RUN-001).
    #[test]
    fn strtol_hex_follows_the_crt() {
        assert_eq!(strtol_hex(b"0011FFFF"), 0x0011_FFFF);
        assert_eq!(strtol_hex(b"  0xFf zz"), 0xFF);
        assert_eq!(strtol_hex(b"-10"), -0x10);
        assert_eq!(strtol_hex(b"+1g"), 1);
        assert_eq!(strtol_hex(b"zz"), 0);
        assert_eq!(strtol_hex(b""), 0);
        assert_eq!(strtol_hex(b"7FFFFFFF"), i32::MAX);
        assert_eq!(strtol_hex(b"80000000"), i32::MAX);
        assert_eq!(strtol_hex(b"FFFFFFFFFFFFFFFFFF"), i32::MAX);
        assert_eq!(strtol_hex(b"-80000000"), i32::MIN);
        assert_eq!(strtol_hex(b"-80000001"), i32::MIN);
    }
    use crate::fixtures;

    /// The original's own 157 draws (P14-RUN-004) are one unbroken LCG sequence.
    #[test]
    fn rand_matches_all_recorded_original_draws() {
        let values: Vec<i32> = fixtures::rows("phase14")
            .iter()
            .filter_map(|row| row.strip_prefix("RAND "))
            .flat_map(str::split_whitespace)
            .map(|value| i32::from_str_radix(value, 16).unwrap())
            .collect();
        assert_eq!(values.len(), 157);
        // Bit 31 of the state never reaches an output, so try bits 0..15 with it clear.
        let matching = (0u32..1 << 16)
            .filter(|low| {
                let mut rand = MsvcRand {
                    holdrand: ((values[0] as u32) << 16) | low,
                };
                values[1..].iter().all(|expected| rand.rand() == *expected)
            })
            .count();
        assert_eq!(
            matching, 1,
            "exactly one state reproduces the whole sequence"
        );
    }

    /// P11-RUN-001's first draw 6C3F. The run recorded `_time64` later, at 0066AEF2, not the seed;
    /// the draw fits that value less 11 seconds and no other seed within 40 seconds of it.
    #[test]
    fn random_next_seeds_once_from_time64() {
        struct Source<F: FnMut() -> i64> {
            rand: MsvcRand,
            time64: F,
        }
        impl<F: FnMut() -> i64> RandomSource for Source<F> {
            fn time64(&mut self) -> i64 {
                (self.time64)()
            }
            fn srand(&mut self, seed: u32) {
                self.rand.srand(seed);
            }
            fn rand(&mut self) -> i32 {
                self.rand.rand()
            }
        }
        let recorded_later = 0x6AC5_B7E7_i64;
        let mut globals = RandomGlobals::default();
        let mut first = Source {
            rand: MsvcRand::default(),
            time64: || recorded_later - 11,
        };
        assert_eq!(random_next(0, &mut globals, &mut first), 0x6C3F);
        assert_eq!(globals.first_use_b069c3, 0);
        let second = first.rand;
        let mut expected = second;
        let mut source = Source {
            rand: second,
            time64: || panic!("time64 is read only once"),
        };
        assert_eq!(random_next(0, &mut globals, &mut source), expected.rand());
    }

    #[test]
    fn stricmp_is_c_locale_and_stops_at_nul() {
        assert_eq!(stricmp(b"Oblivion.ESM", b"oblivion.esm"), 0);
        assert_eq!(stricmp(b"a\0x", b"A\0y"), 0);
        assert_eq!(stricmp(b"a", b"b"), -1);
        assert_eq!(stricmp(b"abc", b"ab"), i32::from(b'c'));
        // 0xC4 and 0xE4 differ in the C locale.
        assert_ne!(stricmp(&[0xC4], &[0xE4]), 0);
        assert_eq!(strnicmp(b"abX", b"ABY", 2), 0);
    }

    #[test]
    fn atol_reads_a_prefix() {
        assert_eq!(atol(b"  -12x"), -12);
        assert_eq!(atol(b"+7"), 7);
        assert_eq!(atol(b""), 0);
        assert_eq!(atol(b"x1"), 0);
    }

    #[test]
    fn scan_f32_reads_the_longest_prefix() {
        assert_eq!(scan_f32(b" 1.5, 2"), Some((1.5, 4)));
        assert_eq!(scan_f32(b"-.5"), Some((-0.5, 3)));
        assert_eq!(scan_f32(b"1e"), Some((1.0, 1)));
        assert_eq!(scan_f32(b"2e2x"), Some((200.0, 3)));
        assert_eq!(scan_f32(b"x"), None);
        assert_eq!(scan_f32(b"."), None);
    }

    #[test]
    fn scan_lists_stop_at_the_first_mismatch() {
        let mut values = [9, 9, 9];
        assert_eq!(scan_u32_list(b"10,20,300", &mut values), 3);
        assert_eq!(values, [10, 20, 300]);
        let mut values = [0, 0, 0];
        assert_eq!(scan_u32_list(b"7", &mut values), 1);
        assert_eq!(values, [7, 0, 0]);
        let (mut first, mut second) = (1.0, 1.0);
        assert_eq!(scan_f32_pair(b"0.25, 0.75", &mut first, &mut second), 2);
        assert_eq!((first, second), (0.25, 0.75));
        // The P03 default "1.0f, 1.0" stops after one field.
        let (mut first, mut second) = (5.0, 5.0);
        assert_eq!(scan_f32_pair(b"1.0f, 1.0", &mut first, &mut second), 1);
        assert_eq!((first, second), (1.0, 5.0));
    }

    #[test]
    fn format_f_uses_six_decimals() {
        assert_eq!(format_f(0.1), "0.100000");
        assert_eq!(format_f(f64::from(1.1f32)), "1.100000");
        assert_eq!(format_f(-2.5), "-2.500000");
    }

    #[test]
    fn text_mode_translates_as_the_crt_does() {
        let mut file = TextModeFile::from_bytes(b"a\r\nb\rc\nlong\x1Aafter".to_vec());
        assert_eq!(file.fgets(0x200).unwrap(), b"a\n");
        assert_eq!(file.fgets(0x200).unwrap(), b"b\rc\n");
        assert_eq!(file.fgets(3).unwrap(), b"lo");
        assert_eq!(file.fgets(0x200).unwrap(), b"ng");
        assert_eq!(file.fgets(0x200), None);
    }
}
