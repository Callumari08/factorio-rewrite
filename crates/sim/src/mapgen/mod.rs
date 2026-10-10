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
    /// Resources with the same order compete: only the most probable is tried on a tile.
    pub order: String,
    /// Tiles kept clear of the same resource around each one (crude oil wells occupy
    /// about 3x3 tiles; ores 0).
    pub spacing: i32,
}

/// A tree, rock or other entity placed by map generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityAutoplace {
    pub entity: EntityProtoId,
    pub probability: NoiseDef,
    /// Entities with the same order compete: only the most probable is tried on a tile.
    pub order: String,
    /// Placement attempts per tile.
    pub placement_density: u32,
    /// Tiles it may be placed on (from collision masks and `tile_restriction`).
    pub allowed_tiles: Vec<bool>,
    /// May sit anywhere within its tile (`placeable-off-grid`).
    pub off_grid: bool,
    /// Keeps this many tiles from other generated entities (`map_generator_bounding_box`).
    pub spacing: i32,
    /// Number of picture variations to choose from.
    pub variations: u8,
}

/// A decorative (grass tuft, decal, small rock): drawn on the ground only, so it is not
/// game state; the client asks for a chunk's decoratives when it draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecorativeAutoplace {
    pub name: String,
    pub probability: NoiseDef,
    pub order: String,
    pub placement_density: u32,
    pub allowed_tiles: Vec<bool>,
    /// Keeps this many tiles from another of the same decorative (`collision_box`).
    pub spacing: i32,
    pub variations: u8,
}

/// A placed decorative: index into the settings' decoratives, position in 1/256 tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacedDecorative {
    pub decorative: u16,
    pub x: i32,
    pub y: i32,
    pub variation: u8,
}

/// Cliff generation: the planet's `cliff_settings` and its cliff entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliffAutoplace {
    pub entity: EntityProtoId,
    /// Orientation names, sorted (their index is a cliff's orientation).
    pub orientations: Vec<String>,
    pub variations: Vec<u8>,
    /// Grid cell size and offset in tiles.
    pub grid_size: [i32; 2],
    pub grid_offset_subtiles: [i32; 2],
    /// Tiles cliffs may stand on.
    pub allowed_tiles: Vec<bool>,
}

/// A cliff the generator placed: its cell centre in 1/256 tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlacedCliff {
    pub x: i32,
    pub y: i32,
    pub orientation: u8,
    pub variation: u8,
}

/// An entity the generator placed: position in 1/256 tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacedEntity {
    pub entity: EntityProtoId,
    pub x: i32,
    pub y: i32,
    pub variation: u8,
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
    /// Trees, rocks and other entities in placement order.
    pub entities: Vec<EntityAutoplace>,
    /// Decoratives in placement order.
    pub decoratives: Vec<DecorativeAutoplace>,
    pub cliffs: Option<CliffAutoplace>,
}

impl Eq for NoiseMapGen {}

/// A compiled [`NoiseMapGen`].
#[derive(Debug)]
pub struct Generator {
    program: Program,
    tiles: Vec<(TileId, NodeId, bool)>,
    resources: Vec<(EntityProtoId, NodeId, NodeId)>,
    resource_orders: Vec<String>,
    resource_spacing: Vec<i32>,
    /// Groups of entities sharing an order string, each with its probability.
    entity_groups: Vec<Vec<(EntityAutoplace, NodeId)>>,
    decoratives: Vec<(DecorativeAutoplace, NodeId)>,
    /// Cliff settings with the `cliff_elevation` and `cliffiness` properties.
    cliffs: Option<(CliffAutoplace, NodeId, NodeId)>,
    cliff_levels: (f32, f32),
    seed: u32,
}

/// Generated contents of one chunk.
pub struct ChunkTerrain {
    pub tiles: Vec<TileId>,
    /// Resource and amount per tile.
    pub resources: Vec<Option<(EntityProtoId, u32)>>,
    pub entities: Vec<PlacedEntity>,
    pub cliffs: Vec<PlacedCliff>,
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
        let mut entity_groups: Vec<Vec<(EntityAutoplace, NodeId)>> = Vec::new();
        for e in &settings.entities {
            let p = c.compile(&e.probability).map_err(|err| format!("entity {}: {err}", e.entity.0))?;
            match entity_groups.last_mut() {
                Some(g) if g[0].0.order == e.order => g.push((e.clone(), p)),
                _ => entity_groups.push(vec![(e.clone(), p)]),
            }
        }
        let mut decoratives = Vec::new();
        for d in &settings.decoratives {
            let p = c.compile(&d.probability).map_err(|err| format!("decorative {}: {err}", d.name))?;
            decoratives.push((d.clone(), p));
        }
        let cliffs = match &settings.cliffs {
            Some(cl) => {
                let elevation = c.compile_name("cliff_elevation").map_err(|e| format!("cliff_elevation: {e}"))?;
                let cliffiness = c.compile_name("cliffiness").map_err(|e| format!("cliffiness: {e}"))?;
                Some((cl.clone(), elevation, cliffiness))
            }
            None => None,
        };
        let number = |k: &str, d: f64| settings.constants.numbers.get(k).copied().unwrap_or(d) as f32;
        let cliff_levels = (number("cliff_elevation_0", 10.0), number("cliff_elevation_interval", 40.0));
        let seed = settings.constants.numbers.get("map_seed").copied().unwrap_or(0.0) as u32;
        let resource_orders = settings.resources.iter().map(|r| r.order.clone()).collect();
        let resource_spacing = settings.resources.iter().map(|r| r.spacing).collect();
        Ok(Generator {
            program: c.program,
            tiles,
            resources,
            resource_orders,
            resource_spacing,
            entity_groups,
            decoratives,
            cliffs,
            cliff_levels,
            seed,
        })
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
        let entity_base = roots.len();
        for g in &self.entity_groups {
            roots.extend(g.iter().map(|e| e.1));
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
            let mut k = 0;
            while k < self.resources.len() {
                // The group of resources sharing this one's order: its most probable member.
                let order = &self.resource_orders[k];
                let end = (k..self.resources.len())
                    .find(|j| &self.resource_orders[*j] != order)
                    .unwrap_or(self.resources.len());
                let prob = |j: usize| values[self.tiles.len() + 2 * j][i];
                let best = (k..end).max_by(|a, b| prob(*a).total_cmp(&prob(*b)).then(b.cmp(a))).unwrap();
                k = end;
                let resource = &self.resources[best].0;
                let p = prob(best);
                if !eval::positive(p) {
                    continue;
                }
                let roll = (basis::hash4(self.seed, xs[i] as i32 as u32, ys[i] as i32 as u32, resource.0 as u32) >> 8)
                    as f32
                    / (1u32 << 24) as f32;
                if roll >= p {
                    continue;
                }
                let amount = values[self.tiles.len() + 2 * best + 1][i];
                // Large resources (oil wells) keep their neighbourhood clear.
                let spacing = self.resource_spacing[best];
                if spacing > 0 {
                    let (x, y) = ((i as i32 % CHUNK_SIZE), (i as i32 / CHUNK_SIZE));
                    let blocked = (-spacing..=spacing).any(|dy| {
                        (-spacing..=spacing).any(|dx| {
                            let (nx, ny) = (x + dx, y + dy);
                            let j = ny * CHUNK_SIZE + nx;
                            (0..CHUNK_SIZE).contains(&nx)
                                && (0..CHUNK_SIZE).contains(&ny)
                                && (j as usize) < i
                                && resources[j as usize].is_some_and(|(r, _)| r == *resource)
                        })
                    });
                    if blocked {
                        continue;
                    }
                }
                if amount >= 1.0 {
                    resources[i] = Some((*resource, amount.min(u32::MAX as f32) as u32));
                    break;
                }
            }
        }
        let entities = self.place_entities(&values[entity_base..], &tiles, first);
        let cliffs = self.place_cliffs(cache, c, &tiles);
        ChunkTerrain { tiles, resources, entities, cliffs }
    }

    /// Cliffs along the contours of `cliff_elevation` at `cliff_elevation_0 + n *
    /// cliff_elevation_interval`, one per grid cell (marching squares), where `cliffiness`
    /// is above 0.5. A cliff's name says which cell sides it runs between with the high
    /// ground on its left (`west_to_east` faces south); it ends (`none`) where the next
    /// cell along the contour is not cliffy.
    fn place_cliffs(&self, cache: &mut SpotCache, c: ChunkPosition, tiles: &[TileId]) -> Vec<PlacedCliff> {
        let Some((cl, elevation, cliffiness)) = &self.cliffs else { return Vec::new() };
        let sub = crate::map::SUBTILES_PER_TILE;
        let [gw, gh] = cl.grid_size;
        let (ox, oy) = (cl.grid_offset_subtiles[0] as f32 / sub as f32, cl.grid_offset_subtiles[1] as f32 / sub as f32);
        let first = c.first_tile();
        // Cells whose top-left corner is in this chunk.
        let (i0, j0) = (first.x.div_euclid(gw), first.y.div_euclid(gh));
        let (ni, nj) = (CHUNK_SIZE / gw, CHUNK_SIZE / gh);
        let corner = |i: i32, j: i32| ((i * gw) as f32 + ox, (j * gh) as f32 + oy);
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for j in j0..=j0 + nj {
            for i in i0..=i0 + ni {
                let (x, y) = corner(i, j);
                xs.push(x);
                ys.push(y);
            }
        }
        let corners = xs.len();
        // Cliffiness at the centres of these cells and the ring around them.
        for j in j0 - 1..=j0 + nj {
            for i in i0 - 1..=i0 + ni {
                let (x, y) = corner(i, j);
                xs.push(x + gw as f32 / 2.0);
                ys.push(y + gh as f32 / 2.0);
            }
        }
        let v = Evaluator { program: &self.program, spots: cache }.eval(&[*elevation, *cliffiness], &xs, &ys);
        let row = (ni + 1) as usize;
        let elev = |i: i32, j: i32| v[0][((j - j0) as usize) * row + (i - i0) as usize];
        let cliffy = |i: i32, j: i32| v[1][corners + ((j - j0 + 1) as usize) * (row + 1) + (i - i0 + 1) as usize] > 0.5;
        let (e0, interval) = self.cliff_levels;
        let mut out = Vec::new();
        for j in j0..j0 + nj {
            for i in i0..i0 + ni {
                if !cliffy(i, j) {
                    continue;
                }
                // Corners clockwise from the top left, with their positions in the cell.
                let e = [elev(i, j), elev(i + 1, j), elev(i + 1, j + 1), elev(i, j + 1)];
                let pos = [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
                let lo = e.iter().copied().fold(f32::INFINITY, f32::min);
                let hi = e.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                if !eval::positive(interval) || !lo.is_finite() || !hi.is_finite() {
                    continue;
                }
                let k = ((lo - e0) / interval).ceil();
                let level = e0 + k * interval;
                if level > hi || level < lo {
                    continue;
                }
                // Sides N, E, S, W between corners (0,1), (1,2), (2,3), (3,0).
                let crossed: Vec<usize> = (0..4).filter(|s| (e[*s] > level) != (e[(s + 1) % 4] > level)).collect();
                if crossed.len() < 2 {
                    continue;
                }
                let (a, b) = (crossed[0], crossed[1]);
                let mid = |s: usize| {
                    let (p, q) = (pos[s], pos[(s + 1) % 4]);
                    ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0)
                };
                let (ma, mb) = (mid(a), mid(b));
                let heading = (mb.0 - ma.0, mb.1 - ma.1);
                let left = (heading.1, -heading.0);
                let high = (0..4).find(|k| e[*k] > level).unwrap();
                let centre = ((ma.0 + mb.0) / 2.0, (ma.1 + mb.1) / 2.0);
                let to_high = (pos[high].0 - centre.0, pos[high].1 - centre.1);
                let (from, to) = if to_high.0 * left.0 + to_high.1 * left.1 > 0.0 { (a, b) } else { (b, a) };
                const SIDES: [&str; 4] = ["north", "east", "south", "west"];
                const STEP: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
                let side_name = |s: usize| {
                    let (dx, dy) = STEP[s];
                    if cliffy(i + dx, j + dy) { SIDES[s] } else { "none" }
                };
                let name = format!("{}_to_{}", side_name(from), side_name(to));
                let Some(orientation) = cl.orientations.iter().position(|o| *o == name) else { continue };
                let (cx, cy) = corner(i, j);
                let (cx, cy) = (cx + gw as f32 / 2.0, cy + gh as f32 / 2.0);
                let (tx, ty) = (cx.floor() as i32, cy.floor() as i32);
                let local = ((ty - first.y) * CHUNK_SIZE + (tx - first.x)) as usize;
                if tiles.get(local).is_some_and(|t| !cl.allowed_tiles.get(t.index()).copied().unwrap_or(false)) {
                    continue;
                }
                let variations = cl.variations.get(orientation).copied().unwrap_or(1).max(1);
                let variation = (self.unit(0xc11f, i as u32, j as u32, 0) * variations as f32) as u8;
                out.push(PlacedCliff {
                    x: (cx * sub as f32).round() as i32,
                    y: (cy * sub as f32).round() as i32,
                    orientation: orientation as u8,
                    variation: variation.min(variations - 1),
                });
            }
        }
        out
    }

    /// The decoratives of a chunk whose tiles are `tiles`. Like entities, decoratives
    /// sharing an order compete for a tile; each keeps clear of its own kind.
    pub fn decoratives(&self, c: ChunkPosition, tiles: &[TileId]) -> Vec<PlacedDecorative> {
        if self.decoratives.is_empty() {
            return Vec::new();
        }
        let first = c.first_tile();
        let n = CHUNK_SIZE as usize;
        let mut xs = Vec::with_capacity(n * n);
        let mut ys = Vec::with_capacity(n * n);
        for dy in 0..CHUNK_SIZE {
            for dx in 0..CHUNK_SIZE {
                xs.push((first.x + dx) as f32);
                ys.push((first.y + dy) as f32);
            }
        }
        let roots: Vec<NodeId> = self.decoratives.iter().map(|d| d.1).collect();
        let mut cache = SpotCache::new();
        let values = Evaluator { program: &self.program, spots: &mut cache }.eval(&roots, &xs, &ys);
        let mut taken = vec![vec![false; n * n]; self.decoratives.len()];
        let mut out = Vec::new();
        let sub = crate::map::SUBTILES_PER_TILE;
        for i in 0..n * n {
            let (lx, ly) = ((i % n) as i32, (i / n) as i32);
            let (tx, ty) = (first.x + lx, first.y + ly);
            let mut k = 0;
            while k < self.decoratives.len() {
                // The group of decoratives sharing this one's order.
                let order = &self.decoratives[k].0.order;
                let end = (k..self.decoratives.len())
                    .find(|j| &self.decoratives[*j].0.order != order)
                    .unwrap_or(self.decoratives.len());
                let best = (k..end).max_by(|a, b| values[*a][i].total_cmp(&values[*b][i]).then(b.cmp(a))).unwrap();
                k = end;
                let (d, _) = &self.decoratives[best];
                let p = values[best][i];
                if !eval::positive(p) || !d.allowed_tiles.get(tiles[i].index()).copied().unwrap_or(false) {
                    continue;
                }
                let r = d.spacing;
                let clear = (-r..=r).all(|dy| {
                    (-r..=r).all(|dx| {
                        let (x, y) = (lx + dx, ly + dy);
                        !(0..n as i32).contains(&x)
                            || !(0..n as i32).contains(&y)
                            || !taken[best][y as usize * n + x as usize]
                    })
                });
                if !clear {
                    continue;
                }
                let salt = 0x0dec_0000 + best as u32;
                if !(0..d.placement_density.max(1)).any(|a| self.unit(salt, tx as u32, ty as u32, a) < p) {
                    continue;
                }
                let jx = (self.unit(salt ^ 1, tx as u32, ty as u32, 9) * sub as f32) as i32;
                let jy = (self.unit(salt ^ 2, tx as u32, ty as u32, 9) * sub as f32) as i32;
                let variation = (self.unit(salt ^ 3, tx as u32, ty as u32, 9) * d.variations.max(1) as f32) as u8;
                out.push(PlacedDecorative {
                    decorative: best as u16,
                    x: tx * sub + jx,
                    y: ty * sub + jy,
                    variation: variation.min(d.variations.saturating_sub(1)),
                });
                taken[best][i] = true;
            }
        }
        out
    }

    fn unit(&self, a: u32, b: u32, c: u32, d: u32) -> f32 {
        (basis::hash4(self.seed ^ a, b, c, d) >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Trees and rocks: on each tile, for each order group, the most probable entity gets
    /// `placement_density` chances of `probability`. At most one per tile, and none within
    /// another's `map_generator_bounding_box`.
    fn place_entities(
        &self,
        values: &[Vec<f32>],
        tiles: &[TileId],
        first: crate::map::TilePosition,
    ) -> Vec<PlacedEntity> {
        let n = CHUNK_SIZE as usize;
        let mut taken = vec![false; n * n];
        let mut out = Vec::new();
        let mut col = 0;
        let mut group_cols = Vec::new();
        for g in &self.entity_groups {
            group_cols.push(col);
            col += g.len();
        }
        for i in 0..n * n {
            let (lx, ly) = ((i % n) as i32, (i / n) as i32);
            let (tx, ty) = (first.x + lx, first.y + ly);
            for (gi, g) in self.entity_groups.iter().enumerate() {
                if taken[i] {
                    break;
                }
                let base = group_cols[gi];
                let mut best = 0;
                for k in 1..g.len() {
                    if values[base + k][i] > values[base + best][i] {
                        best = k;
                    }
                }
                let (e, _) = &g[best];
                let p = values[base + best][i];
                if !eval::positive(p) || !e.allowed_tiles.get(tiles[i].index()).copied().unwrap_or(false) {
                    continue;
                }
                let placed = (0..e.placement_density.max(1))
                    .any(|attempt| self.unit(e.entity.0 as u32, tx as u32, ty as u32, attempt) < p);
                if !placed {
                    continue;
                }
                // Keep clear of entities already placed in this chunk.
                let r = e.spacing;
                let clear = (-r..=r).all(|dy| {
                    (-r..=r).all(|dx| {
                        let (x, y) = (lx + dx, ly + dy);
                        !(0..n as i32).contains(&x)
                            || !(0..n as i32).contains(&y)
                            || !taken[y as usize * n + x as usize]
                    })
                });
                if !clear {
                    continue;
                }
                let sub = crate::map::SUBTILES_PER_TILE;
                let (x, y) = if e.off_grid {
                    // Anywhere in the middle half of the tile.
                    let jx = (self.unit(1, tx as u32, ty as u32, e.entity.0 as u32) * (sub / 2) as f32) as i32;
                    let jy = (self.unit(2, tx as u32, ty as u32, e.entity.0 as u32) * (sub / 2) as f32) as i32;
                    (tx * sub + sub / 4 + jx, ty * sub + sub / 4 + jy)
                } else {
                    (tx * sub + sub / 2, ty * sub + sub / 2)
                };
                let variation =
                    (self.unit(3, tx as u32, ty as u32, e.entity.0 as u32) * e.variations.max(1) as f32) as u8;
                out.push(PlacedEntity {
                    entity: e.entity,
                    x,
                    y,
                    variation: variation.min(e.variations.saturating_sub(1)),
                });
                taken[i] = true;
            }
        }
        out
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
