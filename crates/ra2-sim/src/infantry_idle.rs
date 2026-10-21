//! Verified local Infantry idle arithmetic; it does not execute mission calls.
//!
//! These pure functions preserve normal retail PC53/chop wait arithmetic and
//! signed CDTimer expiry. The callback caller, actual PlayAnim/Scatter effects
//! and Global VoiceIdle stream are separate native contracts; this module is
//! deliberately not wired to the existing skirmish or campaign idle driver.
#[derive(Clone, Copy)]
struct Binary { m: u128, e: i32 }
impl Binary {
    const ZERO: Self = Self { m: 0, e: 0 };
    fn rounded(self) -> Self {
        if self.m == 0 { return Self::ZERO; }
        let bits = 128 - self.m.leading_zeros();
        let shift = bits.saturating_sub(53);
        Self { m: self.m >> shift, e: self.e + shift as i32 }
    }
    fn f64_store(self) -> Option<Self> {
        if self.m == 0 { return Some(Self::ZERO); }
        if self.e + (128 - self.m.leading_zeros()) as i32 - 1 > 1023 { return None; }
        if self.e >= -1074 { return Some(self); }
        let shift = (-1074 - self.e) as u32;
        Some(Self { m: self.m.checked_shr(shift).unwrap_or(0), e: -1074 }.rounded())
    }
    fn from_nonnegative_f64(value: f64) -> Option<Self> {
        let bits = value.to_bits();
        let exp = ((bits >> 52) & 0x7ff) as i32;
        if bits >> 63 != 0 || exp == 0x7ff { return None; }
        let fraction = (bits & ((1u64 << 52) - 1)) as u128;
        Some(if exp == 0 { Self { m: fraction, e: -1074 } } else { Self { m: fraction | (1u128 << 52), e: exp - 1075 } })
    }
    fn times_int(self, n: u32) -> Self { Self { m: self.m * n as u128, e: self.e }.rounded() }
    fn times(self, rhs: Self) -> Self { Self { m: self.m * rhs.m, e: self.e + rhs.e }.rounded() }
    fn combine(self, rhs: Self, subtract: bool) -> Option<Self> {
        if self.m == 0 && !subtract { return Some(rhs); }
        if rhs.m == 0 { return Some(self); }
        let e = self.e.min(rhs.e);
        if (128 - self.m.leading_zeros()) as i32 + self.e - e > 128
            || (128 - rhs.m.leading_zeros()) as i32 + rhs.e - e > 128 { return None; }
        let left = self.m.checked_shl((self.e - e) as u32)?;
        let right = rhs.m.checked_shl((rhs.e - e) as u32)?;
        let m = if subtract { left.checked_sub(right)? } else { left.checked_add(right)? };
        Some(Self { m, e }.rounded())
    }
    fn nonnegative_i32(self) -> Option<i32> {
        if self.m == 0 { return Some(0); }
        if (128 - self.m.leading_zeros()) as i32 + self.e > 31 { return None; }
        let value = if self.e >= 0 { self.m.checked_shl(self.e as u32)? } else { self.m.checked_shr((-self.e) as u32).unwrap_or(0) };
        i32::try_from(value).ok()
    }
}

/// Models finite nonnegative frequencies and non-overflowing positive native durations.
/// None keeps negative/NaN/infinite/overflow or an unrepresentable intermediate explicit.
/// Native conversion returns INT_MIN for invalid/overflow values; those are outside this bounded API.
pub fn idle_wait_pc53_chop(frequency: f64, draw: u32) -> Option<i32> {
    if draw > 0x7fff_fffe { return None; }
    let f = Binary::from_nonnegative_f64(frequency)?;
    let hi = f.times_int(1800).f64_store()?;
    let lo = f.times_int(450).f64_store()?;
    let width = hi.combine(lo, true)?;
    let normal = Binary { m: ((1u128 << 52) + (1 << 22)) * draw as u128, e: -83 }.rounded();
    width.times(normal).combine(lo, false)?.nonnegative_i32()
}


/// Original CDTimer readiness. A stopped timer with a nonzero duration does
/// not expire, including a negative duration. Active elapsed uses signed32
/// subtraction with native wraparound.
pub fn timer_expired(start: i32, duration: i32, current: i32) -> bool {
    if start == -1 { duration == 0 }
    else { current.wrapping_sub(start) >= duration }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_wait_matches_both_original_cpu_arithmetic_profiles() {
        let golden = include_bytes!("../tests/data/infantry_idle_wait_native.bin");
        let mut cases = 0;
        for row in golden.chunks_exact(16) {
            let frequency = f64::from_le_bytes(row[0..8].try_into().unwrap());
            let draw = u32::from_le_bytes(row[8..12].try_into().unwrap());
            let expected = i32::from_le_bytes(row[12..16].try_into().unwrap());
            assert_eq!(idle_wait_pc53_chop(frequency, draw), Some(expected), "frequency={frequency:?}, draw={draw}");
            cases += 1;
        }
        assert_eq!(cases, 112);
        assert_eq!(idle_wait_pc53_chop(0.15, 0x7fff_fffe), Some(269));
        assert_eq!(idle_wait_pc53_chop(1.0, 0x3fff_ffff), Some(1124));
        assert_eq!(idle_wait_pc53_chop(1.0, 0x4000_0000), Some(1125));
    }

    #[test]
    fn idle_wait_keeps_unsupported_duration_profiles_explicit() {
        for value in [-0.1, -0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
            assert_eq!(idle_wait_pc53_chop(value, 0), None, "{value:?}");
        }
        assert_eq!(idle_wait_pc53_chop(0.15, 0x7fff_ffff), None);
        assert_eq!(idle_wait_pc53_chop(0.0, 0), Some(0));
    }

    #[test]
    fn idle_timer_matches_native_before_at_after_stopped_and_future_start_cases() {
        let mut cases = 0;
        for row in include_str!("../tests/data/infantry_idle_timer_native.tsv").lines().filter(|row| !row.starts_with('#')) {
            let cells: Vec<i32> = row.split('\t').map(|value| value.parse().unwrap()).collect();
            assert_eq!(timer_expired(cells[0], cells[1], cells[2]), cells[3] != 0, "{row}");
            cases += 1;
        }
        assert_eq!(cases, 14);
        assert!(!timer_expired(-1, -1, 100));
        assert!(timer_expired(100, -1, 100));
    }
}
