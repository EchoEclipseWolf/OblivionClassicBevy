//! x87 behaviour on SSE. The main thread runs with control word 007F (24-bit precision) after
//! CreateDevice, so a float operation there rounds like Rust's `f32`; expressions the original
//! groups differently stay explicit at their call sites.

/// IDA: `fld dword` / `fstp dword` copies quiet a signaling NaN (bit 22); every other pattern is
/// copied bit for bit. Completion: 100%.
#[must_use]
pub const fn float_copy(bits: u32) -> f32 {
    if bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0 {
        f32::from_bits(bits | 0x0040_0000)
    } else {
        f32::from_bits(bits)
    }
}

/// IDA: `fstp dword` of a value already held as a float. Completion: 100%.
#[must_use]
pub const fn float_store(value: f32) -> f32 {
    float_copy(value.to_bits())
}

/// IDA: an explicit `fstp m32` after a chain held in `f64`: the one store boundary in those expressions.
/// Right only at a 53-bit site (control word 027F: the main thread before CreateDevice, the movie thread).
/// A 24-bit site uses plain `f32` arithmetic instead. Completion: 90% of the source-level store order.
/// TODO: [ ] Compare against captured x87 results.
#[must_use]
pub fn stored_float(value: f64) -> f32 {
    value as f32
}

/// IDA: `__ftol2_sse` / `fistp` under chop: truncation, and the integer indefinite 0x80000000 for
/// NaN and out-of-range values. Rust's `as` saturates, so the range test is required.
/// Completion: 100%.
#[must_use]
pub fn ftol(value: f32) -> i32 {
    if value > -2147483904.0 && value < 2147483648.0 {
        value as i32
    } else {
        i32::MIN
    }
}

/// IDA: `fistp dword` with the round-to-nearest control word; the integer indefinite when out of
/// range. Completion: 100%.
#[must_use]
pub fn fistp(value: f32) -> i32 {
    let rounded = value.round_ties_even();
    if (-2147483648.0..2147483648.0).contains(&rounded) {
        rounded as i32
    } else {
        i32::MIN
    }
}

/// IDA: `fistp qword` under chop: the low dword of the result; the indefinite's low dword is 0.
/// Completion: 100%.
#[must_use]
pub fn ftol64_low(value: f32) -> u32 {
    if value > -9.223_373e18 && value < 9.223_372e18 {
        value as i64 as u32
    } else {
        0
    }
}

/// IDA: `fistp qword` under chop on an x87 double, as `ftol64_low`: the low dword; NaN and overflow
/// store the indefinite 0x8000000000000000, whose low dword is 0. Completion: 100%.
#[must_use]
pub fn ftol64_low_f64(value: f64) -> u32 {
    if (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value) {
        value as i64 as u32
    } else {
        0
    }
}

/// IDA: 007500E2/0075010E RC set to chop before `fistp m32`, then the low word; the indefinite's
/// low word is 0. Completion: 100%.
/// TODO: [ ] Compare x87 precision-control differences for extreme products.
#[must_use]
pub fn truncated_low16(value: f64) -> u16 {
    if !value.is_finite() || !(-2147483648.0..2147483648.0).contains(&value) {
        return 0;
    }
    value.trunc() as i32 as u16
}

/// IDA: 0044F2EE / 0044F305 `fild` of a u32 counter: the signed value, + 2^32 (00A2FC78) when
/// negative, as the 24-bit float sum. Completion: 100%.
#[must_use]
pub fn u32_to_x87_float(value: u32) -> f32 {
    let signed = value as i32;
    let result = signed as f32;
    if signed < 0 {
        result + 4294967296.0
    } else {
        result
    }
}

/// IDA: N/A `fld qword` then `fstp qword` on raw bits: an sNaN comes back quiet; everything else is exact.
#[must_use]
pub const fn fld_fstp_copy_f64(bits: u64) -> u64 {
    const EXPONENT: u64 = 0x7FF0_0000_0000_0000;
    const QUIET: u64 = 0x0008_0000_0000_0000;
    if bits & EXPONENT == EXPONENT && bits & (QUIET - 1) != 0 && bits & QUIET == 0 {
        bits | QUIET
    } else {
        bits
    }
}

/// IDA: the `fld dword` / `fstp dword` pair of a float copy (WinMain 0040E8A2..): x87 `fld` of a signaling NaN raises
/// #IA (masked) and loads the quiet NaN, so the stored bits have bit 22 set; every other value is copied bit for bit.
/// Completion: 100%.
#[must_use]
pub fn fld_fstp_copy(bits: u32) -> f32 {
    let quieted = if (bits & 0x7F80_0000) == 0x7F80_0000 && (bits & 0x007F_FFFF) != 0 {
        bits | 0x0040_0000
    } else {
        bits
    };
    f32::from_bits(quieted)
}

/// IDA: 0065A4E1 `__CIfmod`, float in and out. `fmod` is exact, so the CRT cannot differ. The
/// callers pass the float 2*pi 6.283185482025146. Completion: 100%.
#[must_use]
pub fn float_fmod(value: f32, divisor: f32) -> f32 {
    (f64::from(value) % f64::from(divisor)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_copy_quiets_only_signaling_nans() {
        assert_eq!(float_copy(0x7F80_0001).to_bits(), 0x7FC0_0001);
        assert_eq!(float_copy(0x7FC0_0001).to_bits(), 0x7FC0_0001);
        assert_eq!(float_copy(0x7F80_0000).to_bits(), 0x7F80_0000);
        assert_eq!(float_copy(0x3F80_0000), 1.0);
        assert_eq!(
            float_store(f32::from_bits(0xFF80_0002)).to_bits(),
            0xFFC0_0002
        );
    }

    #[test]
    fn ftol_truncates_and_reports_indefinite() {
        assert_eq!(ftol(1.9), 1);
        assert_eq!(ftol(-1.9), -1);
        assert_eq!(ftol(f32::NAN), i32::MIN);
        assert_eq!(ftol(3e9), i32::MIN);
        assert_eq!(ftol(-3e9), i32::MIN);
    }

    #[test]
    fn fistp_rounds_half_to_even() {
        assert_eq!(fistp(0.5), 0);
        assert_eq!(fistp(1.5), 2);
        assert_eq!(fistp(2.5), 2);
        assert_eq!(fistp(-1.5), -2);
        assert_eq!(fistp(f32::INFINITY), i32::MIN);
    }

    #[test]
    fn wide_truncations_keep_their_low_bits() {
        assert_eq!(truncated_low16(65537.9), 1);
        assert_eq!(truncated_low16(-1.5), 0xFFFF);
        assert_eq!(truncated_low16(3e9), 0);
        assert_eq!(truncated_low16(f64::NAN), 0);
        assert_eq!(ftol64_low(4294967296.0), 0);
        assert_eq!(ftol64_low(5.0), 5);
    }

    /// P12's counters: the x87 load matches the unsigned value at both ends.
    #[test]
    fn counters_convert_as_fild_does() {
        assert_eq!(u32_to_x87_float(114_808), 114_808.0);
        assert_eq!(u32_to_x87_float(0x8000_0000), 2147483648.0);
        assert_eq!(u32_to_x87_float(u32::MAX), 4294967296.0);
    }

    /// RUN-002 / RUN-003 pass through this wrap at Phase 16.
    #[test]
    fn fmod_wraps_with_float_two_pi() {
        const TWO_PI: f32 = 6.283_185_5;
        assert_eq!(TWO_PI.to_bits(), 0x40C9_0FDB);
        assert_eq!(float_fmod(7.0, TWO_PI), (7.0f64 % f64::from(TWO_PI)) as f32);
        assert_eq!(float_fmod(1.0, TWO_PI), 1.0);
    }
}
