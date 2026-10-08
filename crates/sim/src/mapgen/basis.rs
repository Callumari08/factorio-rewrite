//! Noise primitives: gradient (basis) noise, the octave sums built on it, hashing, and
//! the `fastapprox` power function the game uses for `^`.
//!
//! Factorio's basis noise algorithm is not public. Ours is 2D gradient noise with the
//! documented property that integer inputs give 0 (a lattice at integer coordinates), and
//! an amplitude calibrated against the look of the game's maps. Maps therefore follow the
//! game's expressions but are not identical to the game for the same seed.
//!
//! Everything is `f32` like the game, using only IEEE-754 basic operations, so results
//! are the same on every platform.

/// A 32-bit integer hash (lowbias32).
pub fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

pub fn hash4(a: u32, b: u32, c: u32, d: u32) -> u32 {
    hash32(a ^ hash32(b ^ hash32(c ^ hash32(d))))
}

/// CRC-32 (IEEE), used to turn string noise layer names into ids like the game.
pub fn crc32(s: &str) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for b in s.bytes() {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Gradients: the 8 directions of improved Perlin noise.
const GRADIENTS: [(f32, f32); 8] =
    [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0), (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)];

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// The per-layer seed of a noise layer.
pub fn layer_seed(seed0: u32, seed1: u32) -> u32 {
    hash32(seed0 ^ hash32(seed1.wrapping_add(0x9E37_79B9)))
}

fn corner_hash(seed: u32, x: i32, y: i32) -> u32 {
    let mut h = seed ^ (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

/// Single-octave gradient noise at `(x, y)` (already scaled), about -1..1.
pub fn basis(seed0: u32, seed1: u32, x: f32, y: f32) -> f32 {
    basis_seeded(layer_seed(seed0, seed1), x, y)
}

/// [`basis`] with the layer seed already computed.
pub fn basis_seeded(seed: u32, x: f32, y: f32) -> f32 {
    let fx = x.floor();
    let fy = y.floor();
    let (ix, iy) = (fx as i32, fy as i32);
    let (tx, ty) = (x - fx, y - fy);
    let grad = |cx: i32, cy: i32, dx: f32, dy: f32| {
        let (gx, gy) = GRADIENTS[(corner_hash(seed, cx, cy) & 7) as usize];
        gx * dx + gy * dy
    };
    let n00 = grad(ix, iy, tx, ty);
    let n10 = grad(ix + 1, iy, tx - 1.0, ty);
    let n01 = grad(ix, iy + 1, tx, ty - 1.0);
    let n11 = grad(ix + 1, iy + 1, tx - 1.0, ty - 1.0);
    let (u, v) = (fade(tx), fade(ty));
    let a = n00 + (n10 - n00) * u;
    let b = n01 + (n11 - n01) * u;
    (a + (b - a) * v) * BASIS_AMPLITUDE
}

/// Scales raw gradient noise (peak 0.89) so its output spans about -1..1, as the game
/// documents for basis noise.
const BASIS_AMPLITUDE: f32 = 1.12;

/// `fastapprox`'s `fastlog2`.
pub fn fast_log2(x: f32) -> f32 {
    let vx = x.to_bits();
    let mx = f32::from_bits((vx & 0x007F_FFFF) | 0x3f00_0000);
    let y = vx as f32 * 1.192_092_9e-7;
    y - 124.225_52 - 1.498_030_3 * mx - 1.725_88 / (0.352_088_7 + mx)
}

/// `fastapprox`'s `fastpow2`.
pub fn fast_pow2(p: f32) -> f32 {
    let offset = if p < 0.0 { 1.0 } else { 0.0 };
    let clipp = if p < -126.0 { -126.0 } else { p };
    let w = clipp as i32;
    let z = clipp - w as f32 + offset;
    let v = ((1u32 << 23) as f32 * (clipp + 121.274_06 + 27.728_024 / (4.842_525_7 - z) - 1.490_129_1 * z)) as u32;
    f32::from_bits(v)
}

/// The game's `^` operator: `fastapprox`'s `fastpow` (fast, inaccurate).
pub fn fast_pow(x: f32, p: f32) -> f32 {
    if x == 0.0 {
        return if p > 0.0 { 0.0 } else { f32::INFINITY };
    }
    fast_pow2(p * fast_log2(x))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basis_noise_properties() {
        // Zero on the integer lattice, as the game documents.
        assert_eq!(basis(1, 2, 3.0, -4.0), 0.0);
        let mut max = 0.0f32;
        for i in 0..2000 {
            let v = basis(123, 7, i as f32 * 0.137, i as f32 * -0.071);
            max = max.max(v.abs());
        }
        assert!(max > 0.8 && max <= 1.01, "max {max}");
        assert_eq!(basis(5, 6, 0.3, 0.7), basis(5, 6, 0.3, 0.7));
        assert_ne!(basis(5, 6, 0.3, 0.7), basis(5, 7, 0.3, 0.7));
    }

    #[test]
    fn fastapprox_is_close() {
        for (x, p) in [(2.0f32, 3.0f32), (10.0, 0.5), (0.3, 1.7), (100.0, 1.0 / 3.0)] {
            let exact = x.powf(p);
            assert!((fast_pow(x, p) - exact).abs() / exact < 0.01, "{x}^{p}");
        }
        assert_eq!(crc32("123456789"), 0xCBF4_3926);
    }
}
