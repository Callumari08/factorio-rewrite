//! Builds map generation settings from the loaded prototypes.
//!
//! Until Factorio's noise-expression language is implemented, terrain uses the sim's own
//! integer noise. This module maps the available tiles and resources onto that generator:
//! known Nauvis tile names get moisture/aux bands resembling the real map, and any other
//! auto-placed land tiles (from mods) fall back to the remaining space.

use factorio_sim::proto::{EntityData, ItemOrFluid, PrototypeDb, TileId};
use factorio_sim::surface::{LandTileRule, MapGenSettings, ResourceRule};

const N: i64 = 1 << 16;

fn frac(v: f64) -> i64 {
    (v * N as f64) as i64
}

pub fn default_mapgen(db: &PrototypeDb, seed: u64) -> MapGenSettings {
    let water_layer = db.collision_layers.iter().position(|l| l == "water_tile").map(|i| 1u64 << i).unwrap_or(0);
    let is_water = |t: TileId| db.tile(t).collision_mask.layers & water_layer != 0;
    let water =
        db.tile_id("water").or_else(|| db.tile_ids().find(|t| db.tile(*t).fluid.is_some())).unwrap_or(TileId(0));
    let deep_water = db.tile_id("deepwater").unwrap_or(water);

    // (tile, moisture range, aux range) as fractions; the noise centres around 0.5.
    type Band = (&'static str, (f64, f64), (f64, f64));
    let bands: &[Band] = &[
        ("grass-1", (0.58, 2.0), (-1.0, 2.0)),
        ("grass-2", (0.53, 0.58), (-1.0, 2.0)),
        ("grass-3", (0.49, 0.53), (-1.0, 2.0)),
        ("grass-4", (0.46, 0.49), (-1.0, 2.0)),
        ("red-desert-0", (0.42, 0.46), (0.54, 2.0)),
        ("red-desert-1", (0.38, 0.42), (0.54, 2.0)),
        ("red-desert-2", (0.34, 0.38), (0.54, 2.0)),
        ("red-desert-3", (-1.0, 0.34), (0.54, 2.0)),
        ("dirt-4", (0.42, 0.46), (0.47, 0.54)),
        ("dirt-5", (0.38, 0.42), (0.47, 0.54)),
        ("dirt-6", (-1.0, 0.38), (0.47, 0.54)),
        ("dry-dirt", (0.42, 0.46), (-1.0, 0.47)),
        ("sand-1", (0.38, 0.42), (-1.0, 0.47)),
        ("sand-2", (0.34, 0.38), (-1.0, 0.47)),
        ("sand-3", (-1.0, 0.34), (-1.0, 0.47)),
    ];
    let land: Vec<LandTileRule> = bands
        .iter()
        .filter_map(|(name, m, a)| {
            db.tile_id(name).map(|tile| LandTileRule {
                tile,
                moisture: (frac(m.0), frac(m.1)),
                aux: (frac(a.0), frac(a.1)),
            })
        })
        .collect();
    let fallback_land = land
        .first()
        .map(|r| r.tile)
        .or_else(|| db.tile_ids().find(|t| db.tile(*t).autoplace && !is_water(*t)))
        .unwrap_or(TileId(0));

    // Solid resources a character or drill can mine without fluids. Fluid resources and
    // resources needing a mining fluid are not generated yet.
    let resources = db
        .entity_ids()
        .filter(|e| {
            let p = db.entity(*e);
            matches!(p.data, EntityData::Resource { autoplace: true, .. })
                && p.minable.as_ref().is_some_and(|m| {
                    m.required_fluid.is_none() && m.results.iter().all(|r| matches!(r.what, ItemOrFluid::Item(_)))
                })
        })
        .map(|e| ResourceRule { resource: e, starting_area: true, weight: 1 })
        .collect();

    MapGenSettings { seed, water, deep_water, land, fallback_land, resources, starting_radius: 48 }
}
