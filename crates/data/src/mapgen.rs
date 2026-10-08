//! Builds map generation settings from the loaded prototypes.
//!
//! The planet's terrain comes from the game's noise expressions ([`noise_mapgen`]): every
//! named `noise-expression` and `noise-function`, and the `autoplace` of each tile and
//! resource the planet lists. The older banded generator below is kept as a fallback for
//! data whose expressions cannot be compiled.

use std::collections::BTreeMap;

use factorio_sim::mapgen::{
    Constants, DecorativeAutoplace, EntityAutoplace, NoiseDef, NoiseInputs, NoiseMapGen, ResourceAutoplace,
    TileAutoplace,
};
use factorio_sim::proto::{EntityData, ItemOrFluid, PrototypeDb, TileId};
use factorio_sim::surface::{LandTileRule, MapGenSettings, ResourceRule};

use crate::datastage::GameData;
use crate::raw::RawValue;

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

    MapGenSettings { seed, water, deep_water, land, fallback_land, resources, starting_radius: 48, noise: None }
}

/// A `NoiseExpression` value: a string, a number or a boolean.
fn expr_text(v: &RawValue) -> Option<String> {
    match v {
        RawValue::Str(s) => Some(s.clone()),
        RawValue::Nil => None,
        v => v
            .as_bool()
            .map(|b| if b { "1".into() } else { "0".into() })
            .or_else(|| v.as_f64().map(|n| format!("{n:?}"))),
    }
}

/// A noise expression or function with its local expressions and functions.
fn noise_def(p: &RawValue, expression_key: &str) -> Option<NoiseDef> {
    let expression = expr_text(p.get(expression_key))?;
    let locals = p
        .get("local_expressions")
        .as_table()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| expr_text(v).map(|e| (k.clone(), e)))
        .collect();
    let local_functions = p
        .get("local_functions")
        .as_table()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| noise_def(v, "expression").map(|d| (k.clone(), d)))
        .collect();
    let params = p.get("parameters").as_array().iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    Some(NoiseDef { params, expression, locals, local_functions })
}

/// The autoplace expressions of one prototype, with the spec's local definitions.
fn autoplace_defs(a: &RawValue) -> Option<(NoiseDef, Option<NoiseDef>)> {
    let probability = noise_def(a, "probability_expression")?;
    let richness = expr_text(a.get("richness_expression")).map(|e| NoiseDef { expression: e, ..probability.clone() });
    Some((probability, richness))
}

/// Map generation for a planet from the game's noise expressions.
pub fn noise_mapgen(data: &GameData, db: &PrototypeDb, planet: &str, seed: u64) -> NoiseMapGen {
    let mut inputs = NoiseInputs::default();
    for (name, p) in data.raw.get("noise-expression").as_table().into_iter().flatten() {
        if let Some(d) = noise_def(p, "expression") {
            inputs.expressions.insert(name.clone(), d);
        }
    }
    for (name, p) in data.raw.get("noise-function").as_table().into_iter().flatten() {
        if let Some(d) = noise_def(p, "expression") {
            inputs.functions.insert(name.clone(), d);
        }
    }
    let settings = data.prototype("planet", planet).get("map_gen_settings");
    for (k, v) in settings.get("property_expression_names").as_table().into_iter().flatten() {
        if let Some(name) = v.as_str() {
            inputs.property_names.insert(k.clone(), name.to_owned());
        }
    }

    // Constants: map settings at their defaults, and every autoplace control at 100%.
    let map_seed = (seed as u32)
        .wrapping_add(data.prototype("planet", planet).get("map_seed_offset").as_i64().unwrap_or(0) as u32);
    let mut numbers: BTreeMap<String, f64> = BTreeMap::new();
    numbers.insert("map_seed".into(), map_seed as f64);
    numbers.insert("map_seed_small".into(), (map_seed & 0xFFFF) as f64);
    numbers.insert("map_seed_normalized".into(), map_seed as f64 / u32::MAX as f64);
    numbers.insert("map_width".into(), 2_000_000.0);
    numbers.insert("map_height".into(), 2_000_000.0);
    numbers.insert("starting_area_radius".into(), 600.0);
    numbers.insert("peaceful_mode".into(), 0.0);
    numbers.insert("no_enemies_mode".into(), 0.0);
    let cliffs = settings.get("cliff_settings");
    numbers.insert("cliff_elevation_0".into(), cliffs.get("cliff_elevation_0").as_f64().unwrap_or(10.0));
    numbers.insert("cliff_elevation_interval".into(), cliffs.get("cliff_elevation_interval").as_f64().unwrap_or(40.0));
    numbers.insert("cliff_smoothing".into(), cliffs.get("cliff_smoothing").as_f64().unwrap_or(0.0));
    numbers.insert("cliff_richness".into(), cliffs.get("richness").as_f64().unwrap_or(1.0));
    for name in data.raw.get("autoplace-control").as_table().into_iter().flatten().map(|(n, _)| n) {
        for k in ["frequency", "size", "richness"] {
            numbers.insert(format!("control:{name}:{k}"), 1.0);
        }
    }
    for climate in ["moisture", "aux", "temperature"] {
        numbers.insert(format!("control:{climate}:frequency"), 1.0);
        numbers.insert(format!("control:{climate}:bias"), 0.0);
    }
    // One start at the origin and a lake near it. Where the game puts the lake is not
    // documented; this places it 64 tiles away in a direction chosen by the seed.
    let angle =
        (factorio_sim::mapgen::basis::hash32(map_seed ^ 0x1a4e) as f64 / u32::MAX as f64) * std::f64::consts::TAU;
    let mut points = BTreeMap::new();
    points.insert("starting_positions".to_owned(), vec![(0.0f32, 0.0f32)]);
    points
        .insert("starting_lake_positions".to_owned(), vec![((angle.cos() * 64.0) as f32, (angle.sin() * 64.0) as f32)]);
    let constants = Constants { numbers, points };

    let listed = |kind: &str| -> Vec<String> {
        settings
            .get("autoplace_settings")
            .get(kind)
            .get("settings")
            .as_table()
            .into_iter()
            .flatten()
            .map(|(n, _)| n.clone())
            .collect()
    };
    let resource_layer = db.collision_layers.iter().position(|l| l == "resource").map(|i| 1u64 << i).unwrap_or(0);
    let mut tiles = Vec::new();
    for name in listed("tile") {
        let (Some(tile), Some((probability, richness))) =
            (db.tile_id(&name), autoplace_defs(data.prototype("tile", &name).get("autoplace")))
        else {
            continue;
        };
        inputs.variables.insert(format!("tile:{name}:probability"), probability.clone());
        inputs.variables.insert(format!("tile:{name}:richness"), richness.unwrap_or_else(|| probability.clone()));
        let blocks_resources = db.tile(tile).collision_mask.layers & resource_layer != 0;
        tiles.push(TileAutoplace { tile, probability, blocks_resources });
    }
    let mut resources: Vec<(String, ResourceAutoplace)> = Vec::new();
    let mut scenery: Vec<(String, EntityAutoplace)> = Vec::new();
    // Entities the planet lists, plus those whose autoplace control it enables (trees).
    let controls: Vec<String> =
        settings.get("autoplace_controls").as_table().into_iter().flatten().map(|(n, _)| n.clone()).collect();
    let mut names = listed("entity");
    for (kind, name, p) in data.prototypes_in_category("entity") {
        let a = p.get("autoplace");
        if matches!(kind, "tree" | "simple-entity")
            && a.get("control").as_str().is_some_and(|c| controls.iter().any(|x| x == c))
            && !names.iter().any(|n| n == name)
        {
            names.push(name.to_owned());
        }
    }
    for name in names {
        let Some(e) = db.entity_id(&name) else { continue };
        let proto = db.entity(e);
        let Some((kind, _, raw)) = data.prototypes_in_category("entity").find(|(_, n, _)| *n == name) else { continue };
        let Some((probability, richness)) = autoplace_defs(raw.get("autoplace")) else { continue };
        inputs.variables.insert(format!("entity:{name}:probability"), probability.clone());
        inputs
            .variables
            .insert(format!("entity:{name}:richness"), richness.clone().unwrap_or_else(|| probability.clone()));
        // Fluid resources (crude oil) are placed as single entities; not generated yet.
        let solid =
            proto.minable.as_ref().is_some_and(|m| m.results.iter().all(|r| matches!(r.what, ItemOrFluid::Item(_))));
        let order = raw.get("autoplace").get("order").as_str().unwrap_or("").to_owned();
        if kind == "resource" && solid {
            resources.push((order.clone() + &name, ResourceAutoplace { resource: e, probability, richness, order }));
        } else if matches!(kind, "tree" | "simple-entity") {
            let a = raw.get("autoplace");
            let restriction: Vec<&str> =
                a.get("tile_restriction").as_array().iter().filter_map(|t| t.as_str()).collect();
            let allowed_tiles = db
                .tile_ids()
                .map(|t| {
                    let tile = db.tile(t);
                    !tile.collision_mask.collides(&proto.collision_mask)
                        && (restriction.is_empty() || restriction.contains(&tile.name.as_str()))
                })
                .collect();
            let flags: Vec<&str> = raw.get("flags").as_array().iter().filter_map(|f| f.as_str()).collect();
            // Half the size of the generation box, in tiles beyond the entity's own tile.
            let mgbb = match raw.get("map_generator_bounding_box") {
                RawValue::Nil => raw.get("collision_box"),
                b => b,
            };
            let extent = [mgbb.at(0).at(0), mgbb.at(0).at(1), mgbb.at(1).at(0), mgbb.at(1).at(1)]
                .iter()
                .filter_map(|v| v.as_f64())
                .fold(0.0f64, |m, v| m.max(v.abs()));
            let variations =
                raw.get("variations").as_array().len().max(raw.get("pictures").as_array().len()).clamp(1, 255) as u8;
            scenery.push((
                order + &name,
                EntityAutoplace {
                    entity: e,
                    probability,
                    order: raw.get("autoplace").get("order").as_str().unwrap_or("").to_owned(),
                    placement_density: a.get("placement_density").as_i64().unwrap_or(1).max(1) as u32,
                    allowed_tiles,
                    off_grid: flags.contains(&"placeable-off-grid"),
                    spacing: (extent - 0.5).ceil().max(0.0) as i32,
                    variations,
                },
            ));
        }
    }
    resources.sort_by(|a, b| a.0.cmp(&b.0));
    scenery.sort_by(|a, b| a.0.cmp(&b.0));

    // Decoratives: drawn on the ground, kept off tiles that collide with their mask
    // (`doodad` by default, which water has).
    let doodad = db.collision_layers.iter().position(|l| l == "doodad").map(|i| 1u64 << i).unwrap_or(0);
    let layer_bits = |m: &RawValue| -> u64 {
        match m.get("layers").as_table() {
            Some(t) => t
                .iter()
                .filter(|(_, on)| on.as_bool() == Some(true))
                .filter_map(|(n, _)| db.collision_layers.iter().position(|l| l == n))
                .fold(0u64, |acc, i| acc | 1 << i),
            None => doodad,
        }
    };
    let mut decoratives: Vec<(String, DecorativeAutoplace)> = Vec::new();
    for name in listed("decorative") {
        let raw = data.prototype("optimized-decorative", &name);
        let a = raw.get("autoplace");
        let Some((probability, _)) = autoplace_defs(a) else { continue };
        let mask = layer_bits(raw.get("collision_mask"));
        let restriction: Vec<&str> = a.get("tile_restriction").as_array().iter().filter_map(|t| t.as_str()).collect();
        let allowed_tiles = db
            .tile_ids()
            .map(|t| {
                let tile = db.tile(t);
                tile.collision_mask.layers & mask == 0
                    && (restriction.is_empty() || restriction.contains(&tile.name.as_str()))
            })
            .collect();
        let cb = raw.get("collision_box");
        let extent = [cb.at(0).at(0), cb.at(0).at(1), cb.at(1).at(0), cb.at(1).at(1)]
            .iter()
            .filter_map(|v| v.as_f64())
            .fold(0.0f64, |m, v| m.max(v.abs()));
        let order = a.get("order").as_str().unwrap_or("").to_owned();
        decoratives.push((
            order.clone() + &name,
            DecorativeAutoplace {
                name: name.clone(),
                probability,
                order,
                placement_density: a.get("placement_density").as_i64().unwrap_or(1).max(1) as u32,
                allowed_tiles,
                spacing: (extent - 0.5).ceil().max(0.0) as i32,
                variations: raw.get("pictures").as_array().len().clamp(1, 255) as u8,
            },
        ));
    }
    decoratives.sort_by(|a, b| a.0.cmp(&b.0));

    // Cliffs from the planet's cliff settings.
    let cliffs = cliffs.get("name").as_str().and_then(|name| db.entity_id(name)).and_then(|e| {
        let proto = db.entity(e);
        let factorio_sim::proto::EntityData::Cliff { orientations, grid_size, grid_offset } = &proto.data else {
            return None;
        };
        let sub = factorio_sim::map::SUBTILES_PER_TILE;
        Some(factorio_sim::mapgen::CliffAutoplace {
            entity: e,
            orientations: orientations.iter().map(|o| o.name.clone()).collect(),
            variations: orientations.iter().map(|o| o.variations).collect(),
            grid_size: [grid_size[0] / sub, grid_size[1] / sub],
            grid_offset_subtiles: *grid_offset,
            allowed_tiles: db.tile_ids().map(|t| !db.tile(t).collision_mask.collides(&proto.collision_mask)).collect(),
        })
    });
    NoiseMapGen {
        cliffs,
        decoratives: decoratives.into_iter().map(|r| r.1).collect(),
        inputs,
        constants,
        tiles,
        resources: resources.into_iter().map(|r| r.1).collect(),
        entities: scenery.into_iter().map(|r| r.1).collect(),
    }
}

/// [`default_mapgen`] with the planet's noise-expression terrain.
pub fn planet_mapgen(data: &GameData, db: &PrototypeDb, seed: u64) -> MapGenSettings {
    let mut s = default_mapgen(db, seed);
    s.noise = Some(noise_mapgen(data, db, "nauvis", seed));
    s
}
