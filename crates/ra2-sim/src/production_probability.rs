//! Integer evaluation of the original production probability's x87 products.
//! Both constants are exact binary rationals. Keeping their common 2^83
//! denominator avoids borrowing the host CPU's precision or rounding mode.

/// The original comparison for an explicitly known valid x87 control word.
/// Exception flags do not change these finite, nonnegative products.
pub(crate) fn earliest_at_control_word(probability: i32, draw: i32, control: u16) -> Option<bool> {
    if !(0..=0x7fff_fffe).contains(&draw) { return None; }
    if probability <= 0 { return Some(false); }
    let precision = match control & 0x300 { 0 => 24, 0x200 => 53, 0x300 => 64, _ => return None };
    let rounding = (control >> 10) & 3;
    let threshold = (probability as u128 * 5_764_607_523_034_235) << 24;
    let normalized = draw as u128 * ((1u128 << 52) + (1u128 << 22));
    Some(round_product(threshold, precision, rounding) > round_product(normalized, precision, rounding))
}

fn round_product(value: u128, precision: u32, rounding: u16) -> u128 {
    let length = 128 - value.leading_zeros();
    if length <= precision { return value; }
    let shift = length - precision;
    let remainder = value & ((1u128 << shift) - 1);
    let whole = value >> shift;
    let increment = match rounding {
        0 => {
            let half = 1u128 << (shift - 1);
            remainder > half || (remainder == half && whole & 1 != 0)
        }
        2 => remainder != 0, // towards positive infinity
        _ => false,        // positive operands: down and towards zero agree
    };
    (whole + u128::from(increment)) << shift
}

/// Original CRT startup selects PC53, GameMain selects chop and captures that
/// word. Every eligible candidate's nonnull-House Cost calls the original
/// ftol, which restores the capture before this probability tail. The native
/// helper combination and tail were executed for all twelve input modes.
pub(crate) fn earliest_in_campaign(probability: i32, draw: i32) -> bool {
    earliest_at_control_word(probability, draw, 0x0e7f).expect("audited normalized production Range")
}
