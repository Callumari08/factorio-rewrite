//! Fixed-point arithmetic for game logic.

use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// Signed fixed-point number with [`Fixed::FRAC_BITS`] fractional bits, stored in an `i64`.
///
/// All operations are pure integer arithmetic, so results are bit-identical on every
/// platform. Multiplication and division widen to `i128` to avoid intermediate overflow
/// and truncate toward negative infinity (arithmetic shift), consistently everywhere.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Fixed(i64);

impl Fixed {
    pub const FRAC_BITS: u32 = 16;
    pub const ONE: Fixed = Fixed(1 << Self::FRAC_BITS);
    pub const ZERO: Fixed = Fixed(0);

    pub const fn from_raw(raw: i64) -> Self {
        Fixed(raw)
    }

    pub const fn raw(self) -> i64 {
        self.0
    }

    pub const fn from_int(v: i64) -> Self {
        Fixed(v << Self::FRAC_BITS)
    }

    /// Exact rational construction, e.g. `Fixed::from_ratio(1, 32)` for belt speed.
    pub const fn from_ratio(num: i64, den: i64) -> Self {
        Fixed((((num as i128) << Self::FRAC_BITS) / den as i128) as i64)
    }

    /// Converts a float from prototype data. Only call this at load time, never in a tick.
    /// Rounds to nearest so that values written as decimals in Lua map stably.
    pub fn from_f64_at_load(v: f64) -> Self {
        Fixed((v * (1u64 << Self::FRAC_BITS) as f64).round() as i64)
    }

    /// For display only. Never feed the result back into the simulation.
    pub fn to_f64_lossy(self) -> f64 {
        self.0 as f64 / (1u64 << Self::FRAC_BITS) as f64
    }

    pub const fn frac_is_zero(self) -> bool {
        self.0 & ((1 << Self::FRAC_BITS) - 1) == 0
    }

    pub const fn ceil_int(self) -> i64 {
        -((-self.0) >> Self::FRAC_BITS)
    }

    pub fn min(self, o: Fixed) -> Fixed {
        if o < self { o } else { self }
    }

    pub fn max(self, o: Fixed) -> Fixed {
        if o > self { o } else { self }
    }

    pub fn clamp(self, lo: Fixed, hi: Fixed) -> Fixed {
        self.max(lo).min(hi)
    }

    pub const fn abs(self) -> Fixed {
        Fixed(self.0.abs())
    }

    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    /// Multiplies by an integer without rounding.
    pub const fn mul_int(self, v: i64) -> Fixed {
        Fixed(self.0 * v)
    }

    /// Divides by an integer, rounding toward negative infinity.
    pub const fn div_int(self, v: i64) -> Fixed {
        Fixed(self.0.div_euclid(v))
    }

    /// Integer square root of a non-negative value.
    pub fn sqrt(self) -> Fixed {
        assert!(self.0 >= 0);
        let v = (self.0 as u128) << Self::FRAC_BITS;
        Fixed(isqrt(v) as i64)
    }

    pub const fn floor_int(self) -> i64 {
        self.0 >> Self::FRAC_BITS
    }

    pub fn checked_mul(self, rhs: Fixed) -> Option<Fixed> {
        let wide = (self.0 as i128 * rhs.0 as i128) >> Self::FRAC_BITS;
        i64::try_from(wide).ok().map(Fixed)
    }

    pub fn checked_div(self, rhs: Fixed) -> Option<Fixed> {
        if rhs.0 == 0 {
            return None;
        }
        let wide = ((self.0 as i128) << Self::FRAC_BITS).div_euclid(rhs.0 as i128);
        i64::try_from(wide).ok().map(Fixed)
    }
}

fn isqrt(v: u128) -> u128 {
    if v < 2 {
        return v;
    }
    let mut x = v;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + v / x) / 2;
    }
    x
}

impl Add for Fixed {
    type Output = Fixed;
    fn add(self, rhs: Fixed) -> Fixed {
        Fixed(self.0 + rhs.0)
    }
}

impl AddAssign for Fixed {
    fn add_assign(&mut self, rhs: Fixed) {
        self.0 += rhs.0;
    }
}

impl Sub for Fixed {
    type Output = Fixed;
    fn sub(self, rhs: Fixed) -> Fixed {
        Fixed(self.0 - rhs.0)
    }
}

impl SubAssign for Fixed {
    fn sub_assign(&mut self, rhs: Fixed) {
        self.0 -= rhs.0;
    }
}

impl Neg for Fixed {
    type Output = Fixed;
    fn neg(self) -> Fixed {
        Fixed(-self.0)
    }
}

impl Mul for Fixed {
    type Output = Fixed;
    fn mul(self, rhs: Fixed) -> Fixed {
        self.checked_mul(rhs).expect("Fixed multiplication overflow")
    }
}

impl Div for Fixed {
    type Output = Fixed;
    fn div(self, rhs: Fixed) -> Fixed {
        self.checked_div(rhs).expect("Fixed division by zero or overflow")
    }
}

impl fmt::Debug for Fixed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fixed({})", self.to_f64_lossy())
    }
}

impl fmt::Display for Fixed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_f64_lossy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_is_exact() {
        let a = Fixed::from_ratio(1, 32);
        assert_eq!(a * Fixed::from_int(32), Fixed::ONE);
        assert_eq!(Fixed::from_int(3) / Fixed::from_int(2), Fixed::from_ratio(3, 2));
        assert_eq!((-Fixed::ONE / Fixed::from_int(3)).raw(), -21846);
        assert_eq!(Fixed::from_f64_at_load(0.03125), a);
        assert_eq!(Fixed::from_int(9).sqrt(), Fixed::from_int(3));
        assert_eq!(Fixed::from_ratio(3, 2).ceil_int(), 2);
        assert_eq!(Fixed::from_ratio(-3, 2).ceil_int(), -1);
    }
}
