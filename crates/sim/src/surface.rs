//! The map: chunks of tiles and resource deposits, generated deterministically on demand.

use std::collections::BTreeMap;

use crate::map::{CHUNK_SIZE, ChunkPosition, TilePosition};
use crate::noise::{NOISE_ONE, fbm, hash, value_noise};
use crate::proto::{EntityProtoId, PrototypeDb, TileId};
use crate::rng::DetRng;

const CHUNK_TILES: usize = (CHUNK_SIZE * CHUNK_SIZE) as usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceTile {
    pub proto: EntityProtoId,
    pub amount: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Chunk {
    tiles: Vec<TileId>,
    resources: Vec<Option<ResourceTile>>,
    /// Cliffs whose grid cell starts in this chunk.
    cliffs: Vec<crate::mapgen::PlacedCliff>,
}

fn local_index(t: TilePosition) -> usize {
    (t.y.rem_euclid(CHUNK_SIZE) * CHUNK_SIZE + t.x.rem_euclid(CHUNK_SIZE)) as usize
}

/// A band of land tiles: chosen where moisture and "aux" noise fall in the given ranges
/// (both 0..=65536).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandTileRule {
    pub tile: TileId,
    pub moisture: (i64, i64),
    pub aux: (i64, i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceRule {
    pub resource: EntityProtoId,
    /// Gets a guaranteed patch near the spawn point.
    pub starting_area: bool,
    /// Relative frequency of patches outside the starting area.
    pub weight: u32,
}

/// Inputs to map generation. Built by the data loader from the prototypes.
///
/// This is an interim generator using our own integer noise. Factorio's own generator
/// evaluates the noise expressions in the tile and resource `autoplace` definitions; that
/// noise language is not implemented yet, so terrain is plausible but not identical.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapGenSettings {
    pub seed: u64,
    pub water: TileId,
    pub deep_water: TileId,
    pub land: Vec<LandTileRule>,
    pub fallback_land: TileId,
    pub resources: Vec<ResourceRule>,
    /// No water or non-starting resources within this many tiles of the origin.
    pub starting_radius: i32,
    /// The game's noise-expression map generation; when present it replaces the banded
    /// generator above.
    pub noise: Option<crate::mapgen::NoiseMapGen>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Patch {
    resource: EntityProtoId,
    center: TilePosition,
    radius: i32,
    /// Amount at the centre of the patch.
    richness: u32,
    salt: u32,
}

const REGION: i32 = 96;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Surface {
    pub settings: MapGenSettings,
    chunks: BTreeMap<ChunkPosition, Chunk>,
    starting_patches: Vec<Patch>,
    generator: Option<crate::mapgen::GeneratorState>,
    /// Why the noise expressions could not be compiled, if they could not.
    pub noise_error: Option<String>,
    /// Trees and rocks from freshly generated chunks, for the world to add.
    placed: Vec<crate::mapgen::PlacedEntity>,
}

impl Surface {
    pub fn new(settings: MapGenSettings) -> Self {
        let starting_patches = starting_patches(&settings);
        let (generator, noise_error) = match settings.noise.as_ref().map(crate::mapgen::Generator::new) {
            Some(Ok(g)) => (
                Some(crate::mapgen::GeneratorState { generator: std::sync::Arc::new(g), spots: Default::default() }),
                None,
            ),
            Some(Err(e)) => (None, Some(e)),
            None => (None, None),
        };
        Surface { settings, chunks: BTreeMap::new(), starting_patches, generator, noise_error, placed: Vec::new() }
    }

    pub fn chunks(&self) -> impl Iterator<Item = (ChunkPosition, &Chunk)> {
        self.chunks.iter().map(|(p, c)| (*p, c))
    }

    pub fn is_generated(&self, c: ChunkPosition) -> bool {
        self.chunks.contains_key(&c)
    }

    /// Generates the chunk if needed. Generation is a pure function of the seed and the
    /// chunk position, so the order chunks are generated in never matters.
    pub fn ensure_chunk(&mut self, c: ChunkPosition) -> bool {
        if self.chunks.contains_key(&c) {
            return false;
        }
        let chunk = self.generate(c);
        self.chunks.insert(c, chunk);
        true
    }

    /// The decoratives of a generated chunk (computed on demand; not game state).
    pub fn decoratives(&self, c: ChunkPosition) -> Vec<crate::mapgen::PlacedDecorative> {
        match (&self.generator, self.chunks.get(&c)) {
            (Some(g), Some(chunk)) => g.generator.decoratives(c, &chunk.tiles),
            _ => Vec::new(),
        }
    }

    /// Cliffs in generated chunks touching the area (grown by a grid cell).
    pub fn cliffs_near(&self, area: crate::map::Area) -> Vec<crate::mapgen::PlacedCliff> {
        let margin = 8 * crate::map::SUBTILES_PER_TILE;
        let lt = crate::map::MapPosition::new(area.left_top.x - margin, area.left_top.y - margin).tile().chunk();
        let rb =
            crate::map::MapPosition::new(area.right_bottom.x + margin, area.right_bottom.y + margin).tile().chunk();
        let mut out = Vec::new();
        for y in lt.y..=rb.y {
            for x in lt.x..=rb.x {
                if let Some(c) = self.chunks.get(&ChunkPosition { x, y }) {
                    out.extend(c.cliffs.iter().copied());
                }
            }
        }
        out
    }

    /// The cliff entity of these settings, if cliffs are generated.
    pub fn cliff_entity(&self) -> Option<EntityProtoId> {
        self.settings.noise.as_ref().and_then(|n| n.cliffs.as_ref()).map(|c| c.entity)
    }

    /// Entities placed by generation since the last call, in generation order.
    pub fn take_placed_entities(&mut self) -> Vec<crate::mapgen::PlacedEntity> {
        std::mem::take(&mut self.placed)
    }

    pub fn tile(&self, t: TilePosition) -> Option<TileId> {
        self.chunks.get(&t.chunk()).map(|c| c.tiles[local_index(t)])
    }

    pub fn resource(&self, t: TilePosition) -> Option<ResourceTile> {
        self.chunks.get(&t.chunk()).and_then(|c| c.resources[local_index(t)])
    }

    /// Removes `amount` from the resource at `t`; deletes it when depleted (unless infinite).
    pub fn deplete(&mut self, t: TilePosition, amount: u32, infinite: bool) {
        if let Some(chunk) = self.chunks.get_mut(&t.chunk()) {
            let slot = &mut chunk.resources[local_index(t)];
            if let Some(r) = slot {
                r.amount = r.amount.saturating_sub(amount);
                if r.amount == 0 && !infinite {
                    *slot = None;
                }
            }
        }
    }

    pub fn set_tile(&mut self, t: TilePosition, tile: TileId) {
        if let Some(c) = self.chunks.get_mut(&t.chunk()) {
            c.tiles[local_index(t)] = tile;
        }
    }

    pub fn set_resource(&mut self, t: TilePosition, r: Option<ResourceTile>) {
        if let Some(c) = self.chunks.get_mut(&t.chunk()) {
            c.resources[local_index(t)] = r;
        }
    }

    fn generate(&mut self, c: ChunkPosition) -> Chunk {
        if let Some(g) = &mut self.generator {
            let terrain = g.generator.clone().generate(&mut g.spots, c);
            self.placed.extend(terrain.entities);
            return Chunk {
                cliffs: terrain.cliffs,
                tiles: terrain.tiles,
                resources: terrain
                    .resources
                    .into_iter()
                    .map(|r| r.map(|(proto, amount)| ResourceTile { proto, amount }))
                    .collect(),
            };
        }
        let first = c.first_tile();
        let mut tiles = Vec::with_capacity(CHUNK_TILES);
        let mut resources = Vec::with_capacity(CHUNK_TILES);
        let patches = self.patches_near(c);
        for dy in 0..CHUNK_SIZE {
            for dx in 0..CHUNK_SIZE {
                let t = TilePosition::new(first.x + dx, first.y + dy);
                let tile = self.tile_at(t);
                tiles.push(tile);
                let is_water = tile == self.settings.water || tile == self.settings.deep_water;
                resources.push(if is_water { None } else { resource_at(&self.settings, &patches, t) });
            }
        }
        Chunk { tiles, resources, cliffs: Vec::new() }
    }

    fn tile_at(&self, t: TilePosition) -> TileId {
        let s = &self.settings;
        let seed = s.seed;
        let dist2 = t.x as i64 * t.x as i64 + t.y as i64 * t.y as i64;
        let r = s.starting_radius as i64;

        // Elevation: lakes away from spawn, plus one guaranteed lake near spawn.
        let mut elevation = fbm(seed, 1, t.x, t.y, 128, 4) - NOISE_ONE / 2;
        if dist2 < r * r {
            elevation = elevation.max(NOISE_ONE / 8);
        }
        let lake = starting_lake(seed, s.starting_radius);
        let lake_d2 = (t.x - lake.0) as i64 * (t.x - lake.0) as i64 + (t.y - lake.1) as i64 * (t.y - lake.1) as i64;
        let lake_r = 5 + (value_noise(seed, 9, t.x, t.y, 6) * 3 / NOISE_ONE);
        if lake_d2 <= lake_r * lake_r {
            elevation = -NOISE_ONE;
        }
        if elevation < -NOISE_ONE / 4 - NOISE_ONE / 20 {
            return s.deep_water;
        }
        if elevation < -NOISE_ONE / 6 {
            return s.water;
        }

        let moisture = fbm(seed, 2, t.x, t.y, 256, 4);
        let aux = fbm(seed, 3, t.x, t.y, 192, 4);
        s.land
            .iter()
            .find(|r| (r.moisture.0..r.moisture.1).contains(&moisture) && (r.aux.0..r.aux.1).contains(&aux))
            .map(|r| r.tile)
            .unwrap_or(s.fallback_land)
    }

    fn patches_near(&self, c: ChunkPosition) -> Vec<Patch> {
        let mut out = self.starting_patches.clone();
        let center = TilePosition::new(c.x * CHUNK_SIZE + CHUNK_SIZE / 2, c.y * CHUNK_SIZE + CHUNK_SIZE / 2);
        let (rx, ry) = (center.x.div_euclid(REGION), center.y.div_euclid(REGION));
        for gy in ry - 1..=ry + 1 {
            for gx in rx - 1..=rx + 1 {
                out.extend(region_patch(&self.settings, gx, gy));
            }
        }
        out
    }
}

fn starting_lake(seed: u64, starting_radius: i32) -> (i32, i32) {
    let mut rng = DetRng::new(seed ^ 0x1A4E);
    loop {
        let x = rng.below(80) as i32 - 40;
        let y = rng.below(80) as i32 - 40;
        let d2 = x * x + y * y;
        if d2 >= 20 * 20 && d2 <= (starting_radius - 8).max(24).pow(2) {
            return (x, y);
        }
    }
}

fn starting_patches(s: &MapGenSettings) -> Vec<Patch> {
    let mut rng = DetRng::new(s.seed ^ 0x5717);
    let lake = starting_lake(s.seed, s.starting_radius);
    let mut out: Vec<Patch> = Vec::new();
    for (i, rule) in s.resources.iter().filter(|r| r.starting_area).enumerate() {
        let radius = 7 + rng.below(3) as i32;
        let center = loop {
            let x = rng.below(64) as i32 - 32;
            let y = rng.below(64) as i32 - 32;
            let d2 = x * x + y * y;
            let clear_of = |c: TilePosition, r: i32| (x - c.x).pow(2) + (y - c.y).pow(2) > (r + radius + 3).pow(2);
            let near_lake = (x - lake.0).pow(2) + (y - lake.1).pow(2) < (radius + 10).pow(2);
            if (14 * 14..=30 * 30).contains(&d2) && !near_lake && out.iter().all(|p| clear_of(p.center, p.radius)) {
                break TilePosition::new(x, y);
            }
        };
        out.push(Patch { resource: rule.resource, center, radius, richness: 1600, salt: 100 + i as u32 });
    }
    out
}

fn region_patch(s: &MapGenSettings, gx: i32, gy: i32) -> Option<Patch> {
    let h = hash(s.seed, 77, gx, gy);
    if h % 100 >= 45 {
        return None;
    }
    let total: u32 = s.resources.iter().map(|r| r.weight).sum();
    if total == 0 {
        return None;
    }
    let mut pick = (h >> 8) % total;
    let rule = s.resources.iter().find(|r| {
        if pick < r.weight {
            true
        } else {
            pick -= r.weight;
            false
        }
    })?;
    let h2 = hash(s.seed, 78, gx, gy);
    let center = TilePosition::new(
        gx * REGION + 16 + (h2 % (REGION as u32 - 32)) as i32,
        gy * REGION + 16 + ((h2 >> 16) % (REGION as u32 - 32)) as i32,
    );
    let d = ((center.x as i64).pow(2) + (center.y as i64).pow(2)).isqrt() as i32;
    if d < s.starting_radius + 24 {
        return None;
    }
    // Patches grow and get richer with distance, like Factorio's
    // `max((1000 + distance) / 2600, 1)` richness multiplier.
    let radius = (8 + d / 60).min(24);
    let richness = (1600 * (1000 + d as u32) / 2600).max(1600);
    Some(Patch { resource: rule.resource, center, radius, richness, salt: h2 })
}

fn resource_at(s: &MapGenSettings, patches: &[Patch], t: TilePosition) -> Option<ResourceTile> {
    for p in patches {
        let dx = (t.x - p.center.x) as i64;
        let dy = (t.y - p.center.y) as i64;
        let d2 = dx * dx + dy * dy;
        // Wobble the edge with noise so patches are not perfect discs.
        let wobble = value_noise(s.seed, p.salt, t.x, t.y, 5) - NOISE_ONE / 2;
        let r = (p.radius as i64 * NOISE_ONE + wobble * p.radius as i64 * 2 / 5) / NOISE_ONE;
        if r <= 0 || d2 > r * r {
            continue;
        }
        // Amount falls off towards the edge (20% at the rim).
        let falloff = NOISE_ONE - (d2 * NOISE_ONE / (r * r)) * 4 / 5;
        let amount = (p.richness as i64 * falloff / NOISE_ONE).max(1) as u32;
        return Some(ResourceTile { proto: p.resource, amount });
    }
    None
}

/// Checks that generation does not depend on prototypes beyond the ids in the settings.
pub fn validate_settings(db: &PrototypeDb, s: &MapGenSettings) -> bool {
    s.land.iter().all(|r| r.tile.index() < db.tiles.len())
        && s.resources.iter().all(|r| r.resource.index() < db.entities.len())
}
