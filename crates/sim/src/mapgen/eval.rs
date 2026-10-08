//! Evaluates a compiled noise program over a set of points (a chunk's tiles, or spot
//! noise candidate points), one operation at a time over all points.

use std::collections::HashMap;
use std::sync::Arc;

use super::basis::{basis, fast_pow, hash4};
use super::compile::{NodeId, NoiseParams, Op, Program, SpotParams};
use super::parse::BinOp;

/// True for numbers above zero (false for NaN).
pub fn positive(v: f32) -> bool {
    v > 0.0
}

fn bool_f(b: bool) -> f32 {
    if b { 1.0 } else { 0.0 }
}

fn to_i32(v: f32) -> i32 {
    // Saturating, NaN to 0, like a C++ cast in practice.
    v as i32
}

fn pow_int(a: f32, n: i32) -> f32 {
    let mut r = 1.0f32;
    for _ in 0..n.unsigned_abs() {
        r *= a;
    }
    if n < 0 { 1.0 / r } else { r }
}

fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

fn ridge(v: f32, lo: f32, hi: f32) -> f32 {
    let range = hi - lo;
    if range <= 0.0 {
        return lo;
    }
    let period = 2.0 * range;
    let mut t = (v - lo) - ((v - lo) / period).floor() * period;
    if t > range {
        t = period - t;
    }
    lo + t
}

fn binary(op: BinOp, a: f32, b: f32) -> f32 {
    match op {
        BinOp::Pow => fast_pow(a, b),
        BinOp::Mul => a * b,
        BinOp::Div => a / b,
        BinOp::Mod => a - (a / b).floor() * b,
        BinOp::Rem => libm::fmodf(a, b),
        BinOp::Add => a + b,
        BinOp::Sub => a - b,
        BinOp::Lt => bool_f(a < b),
        BinOp::Le => bool_f(a <= b),
        BinOp::Gt => bool_f(a > b),
        BinOp::Ge => bool_f(a >= b),
        BinOp::Eq => bool_f(a == b),
        BinOp::Ne => bool_f(a != b),
        BinOp::BitAnd => (to_i32(a) & to_i32(b)) as f32,
        BinOp::BitXor => (to_i32(a) ^ to_i32(b)) as f32,
        BinOp::BitOr => (to_i32(a) | to_i32(b)) as f32,
    }
}

/// Octave sums. `persistence` is per point for variable-persistence noise.
fn octaves(op: &Op, p: &NoiseParams, x: f32, y: f32, persistence: f32) -> f32 {
    let (x, y) = (x + p.offset_x, y + p.offset_y);
    match op {
        Op::Basis(_) => p.output_scale * basis(p.seed0, p.seed1, x * p.input_scale, y * p.input_scale),
        Op::Multioctave(_) | Op::VariablePersistence(_) => {
            // Octave 0 is the largest; each next one has twice the frequency and
            // `persistence` times the amplitude.
            let pers = if matches!(op, Op::Multioctave(_)) { p.persistence } else { persistence };
            let mut amp = p.output_scale;
            if matches!(op, Op::VariablePersistence(_)) {
                amp *= pow_int(2.0, p.octaves as i32);
            }
            let mut scale = p.input_scale;
            let mut sum = 0.0;
            for k in 0..p.octaves {
                sum += amp * basis(p.seed0.wrapping_add(k), p.seed1, x * scale, y * scale);
                amp *= pers;
                scale *= 2.0;
            }
            sum
        }
        Op::QuickMultioctave(_) => {
            let mut amp = p.output_scale;
            let mut scale = p.input_scale;
            let mut sum = 0.0;
            for k in 0..p.octaves {
                sum += amp * basis(p.seed0.wrapping_add(k * p.octave_seed0_shift), p.seed1, x * scale, y * scale);
                amp *= p.octave_output_scale_multiplier;
                scale *= p.octave_input_scale_multiplier;
            }
            sum
        }
        _ => 0.0,
    }
}

fn unit_random(seed: u32, x: f32, y: f32) -> f32 {
    (hash4(seed, x.to_bits(), y.to_bits(), 0x2545_f491) >> 8) as f32 / (1u32 << 24) as f32
}

/// One operation on scalar arguments (also used for constant folding).
pub fn scalar(op: &Op, a: &[f32]) -> f32 {
    match op {
        Op::Const(v) => *v as f32,
        Op::X | Op::Y | Op::Spot(_) => 0.0,
        Op::Neg => -a[0],
        Op::BitNot => !to_i32(a[0]) as f32,
        Op::Bin(b) => binary(*b, a[0], a[1]),
        Op::PowInt(n) => pow_int(a[0], *n),
        Op::PowPrecise => libm::powf(a[0], a[1]),
        Op::Sqrt => a[0].sqrt(),
        Op::Abs => a[0].abs(),
        Op::Floor => a[0].floor(),
        Op::Ceil => a[0].ceil(),
        Op::Sin => libm::sinf(a[0]),
        Op::Cos => libm::cosf(a[0]),
        Op::Log2 => libm::log2f(a[0]),
        Op::Atan2 => libm::atan2f(a[0], a[1]),
        Op::Clamp => clamp(a[0], a[1], a[2]),
        Op::Ridge => ridge(a[0], a[1], a[2]),
        Op::Min => a.iter().copied().fold(f32::INFINITY, f32::min),
        Op::Max => a.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        Op::If => {
            if a[0] > 0.0 {
                a[1]
            } else {
                a[2]
            }
        }
        Op::Basis(p) | Op::Multioctave(p) | Op::QuickMultioctave(p) => octaves(op, p, a[0], a[1], 0.0),
        Op::VariablePersistence(p) => octaves(op, p, a[0], a[1], a[2]),
        Op::RandomPenalty { seed, amplitude } => {
            if a[2] > 0.0 {
                a[2] - amplitude * unit_random(*seed, a[0], a[1])
            } else {
                a[2]
            }
        }
        Op::NearestPoint { points, maximum_distance, mode } => {
            let mut best = (f32::INFINITY, 0.0, 0.0);
            for (px, py) in points.iter() {
                let (dx, dy) = (a[0] - px, a[1] - py);
                let d = (dx * dx + dy * dy).sqrt();
                if d < best.0 {
                    best = (d, dx, dy);
                }
            }
            match mode {
                0 => best.0.min(*maximum_distance),
                1 => best.1,
                _ => best.2,
            }
        }
        Op::ExpressionInRange { peak_multiplier, peak_maximum, from, to } => {
            let mut m = f32::INFINITY;
            for (i, v) in a.iter().enumerate() {
                m = m.min((v - from[i]).min(to[i] - v));
            }
            (m * peak_multiplier).min(*peak_maximum)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub height: f32,
}

/// Spots per (spot noise operation, region), a pure function of the program.
pub type SpotCache = HashMap<(NodeId, i32, i32), Arc<Vec<Spot>>>;

pub struct Evaluator<'a> {
    pub program: &'a Program,
    pub spots: &'a mut SpotCache,
}

impl Evaluator<'_> {
    /// Values of each root at each point.
    pub fn eval(&mut self, roots: &[NodeId], xs: &[f32], ys: &[f32]) -> Vec<Vec<f32>> {
        let program = self.program;
        let n = program.nodes.len();
        let mut needed = vec![false; n];
        let mut stack: Vec<NodeId> = roots.to_vec();
        while let Some(id) = stack.pop() {
            if std::mem::replace(&mut needed[id as usize], true) {
                continue;
            }
            stack.extend(program.nodes[id as usize].args.iter().copied());
        }
        let len = xs.len();
        let mut values: Vec<Option<Vec<f32>>> = vec![None; n];
        let mut args: Vec<f32> = Vec::new();
        for id in 0..n {
            if !needed[id] {
                continue;
            }
            let node = &program.nodes[id];
            let out: Vec<f32> = match &node.op {
                Op::X => xs.to_vec(),
                Op::Y => ys.to_vec(),
                Op::Const(v) => vec![*v as f32; len],
                Op::Spot(p) => {
                    let (x, y) = (
                        values[node.args[0] as usize].clone().unwrap(),
                        values[node.args[1] as usize].clone().unwrap(),
                    );
                    self.spot_values(id as NodeId, p, &x, &y)
                }
                op => {
                    let inputs: Vec<&Vec<f32>> =
                        node.args.iter().map(|a| values[*a as usize].as_ref().unwrap()).collect();
                    (0..len)
                        .map(|i| {
                            args.clear();
                            args.extend(inputs.iter().map(|v| v[i]));
                            scalar(op, &args)
                        })
                        .collect()
                }
            };
            values[id] = Some(out);
        }
        roots.iter().map(|r| values[*r as usize].clone().unwrap()).collect()
    }

    fn spot_values(&mut self, id: NodeId, p: &SpotParams, xs: &[f32], ys: &[f32]) -> Vec<f32> {
        let reach = p.maximum_spot_basement_radius.max(1.0);
        let r = p.region_size.max(1.0);
        let mut out = Vec::with_capacity(xs.len());
        let mut regions: Vec<(i32, i32, Arc<Vec<Spot>>)> = Vec::new();
        for (x, y) in xs.iter().zip(ys) {
            let mut v = p.basement_value;
            for ry in ((y - reach) / r).floor() as i32..=((y + reach) / r).floor() as i32 {
                for rx in ((x - reach) / r).floor() as i32..=((x + reach) / r).floor() as i32 {
                    let spots = match regions.iter().find(|(a, b, _)| *a == rx && *b == ry) {
                        Some((_, _, s)) => s.clone(),
                        None => {
                            let s = self.region_spots(id, p, rx, ry);
                            regions.push((rx, ry, s.clone()));
                            s
                        }
                    };
                    for s in spots.iter() {
                        let (dx, dy) = (x - s.x, y - s.y);
                        let d2 = dx * dx + dy * dy;
                        if d2 >= reach * reach {
                            continue;
                        }
                        let cone = s.height * (1.0 - d2.sqrt() / s.radius);
                        if cone > v {
                            v = cone;
                        }
                    }
                }
            }
            out.push(v);
        }
        out
    }

    /// The spots of one region: random candidate points (shared by every spot noise with
    /// the same seeds, size and spacing; `skip_offset`/`skip_span` pick this one's share),
    /// taken in order of favourability until the region's target quantity is reached.
    fn region_spots(&mut self, id: NodeId, p: &SpotParams, rx: i32, ry: i32) -> Arc<Vec<Spot>> {
        if let Some(s) = self.spots.get(&(id, rx, ry)) {
            return s.clone();
        }
        let r = p.region_size.max(1.0);
        let seed = hash4(p.seed0, p.seed1, r.to_bits(), p.spacing.to_bits());
        let mut counter = 0u32;
        let mut next = || {
            counter += 1;
            (hash4(seed, rx as u32, ry as u32, counter) >> 8) as f32 / (1u32 << 24) as f32
        };
        let mut points: Vec<(f32, f32)> = Vec::new();
        for _ in 0..p.candidate_points {
            let mut candidate = (0.0, 0.0);
            for _attempt in 0..10 {
                candidate = (rx as f32 * r + next() * r, ry as f32 * r + next() * r);
                let spaced = points.iter().all(|(px, py)| {
                    let (dx, dy) = (px - candidate.0, py - candidate.1);
                    dx * dx + dy * dy >= p.spacing * p.spacing
                });
                if spaced {
                    break;
                }
            }
            points.push(candidate);
        }
        let mine: Vec<(f32, f32)> =
            points.into_iter().skip(p.skip_offset as usize).step_by(p.skip_span.max(1) as usize).collect();
        let xs: Vec<f32> = mine.iter().map(|p| p.0).collect();
        let ys: Vec<f32> = mine.iter().map(|p| p.1).collect();
        let v = self.eval(&[p.density, p.quantity, p.radius, p.favorability], &xs, &ys);
        let (density, quantity, radius, favor) = (&v[0], &v[1], &v[2], &v[3]);
        let mut spots = Vec::new();
        if !mine.is_empty() {
            let mean = density.iter().map(|d| d.max(0.0)).sum::<f32>() / mine.len() as f32;
            let target = mean * r * r;
            let mut order: Vec<usize> = (0..mine.len()).collect();
            order.sort_by(|a, b| favor[*b].total_cmp(&favor[*a]).then(a.cmp(b)));
            let mut total = 0.0f32;
            for i in order {
                if total >= target {
                    break;
                }
                let mut q = quantity[i];
                let rad = radius[i];
                if !positive(q) || !positive(rad) {
                    continue;
                }
                if p.hard_region_target_quantity && total + q > target {
                    q = target - total;
                }
                total += q;
                let height = 3.0 * q / (std::f32::consts::PI * rad * rad);
                spots.push(Spot { x: mine[i].0, y: mine[i].1, radius: rad, height });
            }
        }
        let spots = Arc::new(spots);
        self.spots.insert((id, rx, ry), spots.clone());
        spots
    }
}
