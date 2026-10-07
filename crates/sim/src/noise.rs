//! Integer-only noise for map generation, so every peer generates identical terrain.

/// Noise values are in `0..=NOISE_ONE`.
pub const NOISE_ONE: i64 = 1 << 16;

/// Stateless 32-bit hash of a lattice point.
pub fn hash(seed: u64, salt: u32, x: i32, y: i32) -> u32 {
    let mut z = seed
        ^ (salt as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (x as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (y as u32 as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) as u32
}

fn smooth(t: i64) -> i64 {
    // 3t² - 2t³ in 16-bit fixed point.
    let t2 = (t * t) >> 16;
    let t3 = (t2 * t) >> 16;
    3 * t2 - 2 * t3
}

/// Smoothly interpolated value noise with lattice spacing `cell` tiles, sampled at tile
/// coordinates. Returns `0..=NOISE_ONE`.
pub fn value_noise(seed: u64, salt: u32, x: i32, y: i32, cell: i32) -> i64 {
    let (cx, cy) = (x.div_euclid(cell), y.div_euclid(cell));
    let tx = smooth((x.rem_euclid(cell) as i64 * NOISE_ONE) / cell as i64);
    let ty = smooth((y.rem_euclid(cell) as i64 * NOISE_ONE) / cell as i64);
    let corner = |dx: i32, dy: i32| (hash(seed, salt, cx + dx, cy + dy) >> 16) as i64;
    let top = corner(0, 0) + (((corner(1, 0) - corner(0, 0)) * tx) >> 16);
    let bottom = corner(0, 1) + (((corner(1, 1) - corner(0, 1)) * tx) >> 16);
    top + (((bottom - top) * ty) >> 16)
}

/// Fractal sum of `octaves` value-noise layers, halving cell size and amplitude each
/// octave. Returns `0..=NOISE_ONE`.
pub fn fbm(seed: u64, salt: u32, x: i32, y: i32, cell: i32, octaves: u32) -> i64 {
    let mut sum = 0;
    let mut norm = 0;
    let mut amp = 1 << octaves;
    let mut c = cell;
    for o in 0..octaves {
        sum += value_noise(seed, salt.wrapping_add(o * 7919), x, y, c.max(1)) * amp;
        norm += amp;
        amp /= 2;
        c /= 2;
    }
    sum / norm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_stable_and_in_range() {
        let a = fbm(42, 1, 100, -37, 64, 4);
        assert_eq!(a, fbm(42, 1, 100, -37, 64, 4));
        for i in -50..50 {
            let v = fbm(7, 3, i * 13, i * -7, 32, 3);
            assert!((0..=NOISE_ONE).contains(&v));
        }
    }
}
