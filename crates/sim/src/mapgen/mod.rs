//! Factorio's noise-expression map generation.
//!
//! The game describes its maps with noise expressions in the prototype data (named
//! `noise-expression`s and `noise-function`s, and each tile's and resource's `autoplace`).
//! This module parses ([`parse`]), compiles ([`compile`]) and evaluates ([`eval`]) them,
//! so terrain and resources follow the game's own rules and mods' changes.
//!
//! Map generation is the one place the simulation uses floating point: the game computes
//! noise in `f32`, and so do we, with only IEEE-754 basic operations and the pure-Rust
//! `libm`, which give the same bits on every platform.

pub mod basis;
pub mod compile;
pub mod eval;
pub mod parse;

use std::sync::Arc;

use compile::{Compiler, NodeId, Program};
pub use compile::{Constants, NoiseDef, NoiseInputs};
use eval::{Evaluator, SpotCache};

use crate::map::{CHUNK_SIZE, ChunkPosition};
use crate::proto::{EntityProtoId, TileId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileAutoplace {
    pub tile: TileId,
    pub probability: NoiseDef,
    /// Resources cannot be placed on this tile (water).
    pub blocks_resources: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceAutoplace {
    pub resource: EntityProtoId,
    pub probability: NoiseDef,
    pub richness: Option<NoiseDef>,
}

/// Everything needed to generate a planet's terrain from noise expressions.
#[derive(Clone, Debug, PartialEq)]
pub struct NoiseMapGen {
    pub inputs: NoiseInputs,
    pub constants: Constants,
    /// Tiles in prototype order; the most probable one is placed.
    pub tiles: Vec<TileAutoplace>,
    /// Resources in placement order (`order`, then name).
    pub resources: Vec<ResourceAutoplace>,
}

impl Eq for NoiseMapGen {}

/// A compiled [`NoiseMapGen`].
#[derive(Debug)]
pub struct Generator {
    program: Program,
    tiles: Vec<(TileId, NodeId, bool)>,
    resources: Vec<(EntityProtoId, NodeId, NodeId)>,
    seed: u32,
}

/// Generated contents of one chunk.
pub struct ChunkTerrain {
    pub tiles: Vec<TileId>,
    /// Resource and amount per tile.
    pub resources: Vec<Option<(EntityProtoId, u32)>>,
}

impl Generator {
    pub fn new(settings: &NoiseMapGen) -> Result<Generator, String> {
        let mut c = Compiler::new(&settings.inputs, &settings.constants);
        let mut tiles = Vec::new();
        for t in &settings.tiles {
            let id = c.compile(&t.probability).map_err(|e| format!("tile {}: {e}", t.tile.0))?;
            tiles.push((t.tile, id, t.blocks_resources));
        }
        let mut resources = Vec::new();
        for r in &settings.resources {
            let p = c.compile(&r.probability).map_err(|e| format!("resource {}: {e}", r.resource.0))?;
            let rich = match &r.richness {
                Some(def) => c.compile(def).map_err(|e| format!("resource {} richness: {e}", r.resource.0))?,
                None => p,
            };
            resources.push((r.resource, p, rich));
        }
        let seed = settings.constants.numbers.get("map_seed").copied().unwrap_or(0.0) as u32;
        Ok(Generator { program: c.program, tiles, resources, seed })
    }

    pub fn node_count(&self) -> usize {
        self.program.nodes.len()
    }

    pub fn generate(&self, cache: &mut SpotCache, c: ChunkPosition) -> ChunkTerrain {
        let first = c.first_tile();
        let n = (CHUNK_SIZE * CHUNK_SIZE) as usize;
        let mut xs = Vec::with_capacity(n);
        let mut ys = Vec::with_capacity(n);
        for dy in 0..CHUNK_SIZE {
            for dx in 0..CHUNK_SIZE {
                xs.push((first.x + dx) as f32);
                ys.push((first.y + dy) as f32);
            }
        }
        let mut roots: Vec<NodeId> = self.tiles.iter().map(|t| t.1).collect();
        for r in &self.resources {
            roots.push(r.1);
            roots.push(r.2);
        }
        let mut ev = Evaluator { program: &self.program, spots: cache };
        let values = ev.eval(&roots, &xs, &ys);

        let mut tiles = Vec::with_capacity(n);
        let mut resources = vec![None; n];
        for i in 0..n {
            // The most probable tile wins; ties go to the earlier one.
            let mut best = 0;
            for t in 1..self.tiles.len() {
                if values[t][i] > values[best][i] {
                    best = t;
                }
            }
            tiles.push(self.tiles.get(best).map(|t| t.0).unwrap_or(TileId(0)));
            if self.tiles.get(best).is_some_and(|t| t.2) {
                continue;
            }
            for (k, (resource, _, _)) in self.resources.iter().enumerate() {
                let p = values[self.tiles.len() + 2 * k][i];
                if !eval::positive(p) {
                    continue;
                }
                let roll = (basis::hash4(self.seed, xs[i] as i32 as u32, ys[i] as i32 as u32, resource.0 as u32) >> 8)
                    as f32
                    / (1u32 << 24) as f32;
                if roll >= p {
                    continue;
                }
                let amount = values[self.tiles.len() + 2 * k + 1][i];
                if amount >= 1.0 {
                    resources[i] = Some((*resource, amount.min(u32::MAX as f32) as u32));
                    break;
                }
            }
        }
        ChunkTerrain { tiles, resources }
    }
}

/// A compiled generator plus its spot cache. Derived from the settings, so it is not
/// part of the game state and compares equal to any other.
#[derive(Clone)]
pub struct GeneratorState {
    pub generator: Arc<Generator>,
    pub spots: SpotCache,
}

impl PartialEq for GeneratorState {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for GeneratorState {}

impl std::fmt::Debug for GeneratorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Generator({} operations)", self.generator.node_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn eval1(src: &str, x: f32, y: f32) -> f32 {
        let mut inputs = NoiseInputs::default();
        inputs.functions.insert(
            "lerp".into(),
            NoiseDef {
                params: vec!["a".into(), "b".into(), "t".into()],
                expression: "a + (b - a) * t".into(),
                ..Default::default()
            },
        );
        inputs.expressions.insert(
            "ten".into(),
            NoiseDef {
                expression: "five * 2".into(),
                locals: BTreeMap::from([("five".into(), "5".into())]),
                ..Default::default()
            },
        );
        let mut constants = Constants::default();
        constants.numbers.insert("map_seed".into(), 123.0);
        constants.points.insert("starting_positions".into(), vec![(0.0, 0.0)]);
        let mut c = Compiler::new(&inputs, &constants);
        let root = c.compile(&NoiseDef::expr(src)).unwrap();
        let mut cache = SpotCache::new();
        Evaluator { program: &c.program, spots: &mut cache }.eval(&[root], &[x], &[y])[0][0]
    }

    #[test]
    fn expressions_evaluate() {
        assert_eq!(eval1("1 + 2 * 3", 0.0, 0.0), 7.0);
        assert_eq!(eval1("x * 10 + y", 3.0, 4.0), 34.0);
        assert_eq!(eval1("lerp(2, 4, 0.5)", 0.0, 0.0), 3.0);
        assert_eq!(eval1("lerp{t = 1, a = 0, b = x}", 7.0, 0.0), 7.0);
        assert_eq!(eval1("ten + var('ten')", 0.0, 0.0), 20.0);
        assert_eq!(eval1("clamp(x, -1, 1) + min(1, 2, 3) + max(4, y)", 5.0, 2.0), 1.0 + 1.0 + 4.0);
        assert_eq!(eval1("if(x > 1, 10, 20)", 2.0, 0.0), 10.0);
        assert_eq!(eval1("(x < 1) + (x == 2) * 2", 2.0, 0.0), 2.0);
        assert_eq!(eval1("x ^ 2", 3.0, 0.0), 9.0);
        assert_eq!(eval1("-5 % 3", 0.0, 0.0), 1.0);
        assert_eq!(eval1("-5 %% 3", 0.0, 0.0), -2.0);
        assert_eq!(eval1("distance_from_nearest_point{x = x, y = y, points = starting_positions}", 3.0, 4.0), 5.0);
        assert_eq!(eval1("expression_in_range(10, 1, x, 0, 1)", 0.5, 0.0), 1.0);
        assert!((eval1("expression_in_range(10, 1, x, 0, 1)", 0.95, 0.0) - 0.5).abs() < 1e-5);
        assert_eq!(eval1("basis_noise{x = x, y = y, seed0 = map_seed, seed1 = 0}", 3.0, 7.0), 0.0);
        assert!(eval1("multioctave_noise{x = x, y = y, seed0 = map_seed, seed1 = 1, octaves = 4, persistence = 0.5, input_scale = 1/7}", 3.0, 7.0).abs() < 2.0);
        let r = eval1("random_penalty{x = x, y = y, source = 5, amplitude = 2}", 1.0, 2.0);
        assert!((3.0..=5.0).contains(&r));
    }

    #[test]
    fn spot_noise_makes_cones() {
        let src = "spot_noise{x = x, y = y, density_expression = 0.05, spot_quantity_expression = 1000, \
                   spot_radius_expression = 10, spot_favorability_expression = 1, seed0 = map_seed, seed1 = 3, \
                   basement_value = -1, maximum_spot_basement_radius = 30, region_size = 256, candidate_spot_count = 30}";
        let mut peak = f32::NEG_INFINITY;
        for y in (0..256).step_by(4) {
            for x in (0..256).step_by(4) {
                peak = peak.max(eval1(src, x as f32, y as f32));
            }
        }
        // Target 0.05 * 256² = 3277: three full spots. Height of a 1000-quantity, radius-10
        // cone: 3 * 1000 / (pi * 100) = 9.5.
        assert!(peak > 5.0 && peak < 9.6, "peak {peak}");
    }
}
