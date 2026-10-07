//! Typed, simulation-ready prototypes.
//!
//! These are produced from Factorio's Lua data stage by the `factorio-data` crate, with
//! every float converted to fixed point at load time. Everything is keyed by name in a
//! `BTreeMap` so iteration order is identical on every machine, and every type is open
//! for extension by mods (no enum of base-game names anywhere).

use std::collections::BTreeMap;

use crate::fixed::Fixed;
use crate::map::{MapPosition, SUBTILES_PER_TILE, TilePosition};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemProto {
    pub name: String,
    /// Prototype `type`, e.g. `item`, `tool`, `ammo`, `item-with-entity-data`.
    pub kind: String,
    pub stack_size: u32,
    pub place_result: Option<String>,
    pub fuel_value_joules: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeIngredient {
    /// `item` or `fluid`.
    pub kind: String,
    pub name: String,
    pub amount: Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeProto {
    pub name: String,
    pub category: String,
    /// Crafting time in ticks at crafting speed 1, rounded to the nearest tick.
    pub energy_required_ticks: u32,
    pub ingredients: Vec<RecipeIngredient>,
    pub results: Vec<RecipeIngredient>,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityProto {
    pub name: String,
    /// Prototype `type`, e.g. `transport-belt`, `assembling-machine`.
    pub kind: String,
    /// Collision box in 1/256-tile units, relative to the entity position.
    pub collision_box: [[i32; 2]; 2],
    /// Belt speed in tiles per tick, for `transport-belt`-like prototypes.
    pub belt_speed: Option<Fixed>,
}

impl EntityProto {
    /// Footprint in whole tiles, rounding the collision box up (a 0.8x0.8 box is 1x1).
    pub fn tile_size(&self) -> (i32, i32) {
        let [[l, t], [r, b]] = self.collision_box;
        let tiles = |span: i32| ((span + SUBTILES_PER_TILE - 1) / SUBTILES_PER_TILE).max(1);
        (tiles(r - l), tiles(b - t))
    }

    /// Where an entity covering the tile at `tile` (its top-left tile) is centred.
    pub fn position_for_tile(&self, tile: TilePosition) -> MapPosition {
        let (w, h) = self.tile_size();
        MapPosition {
            x: tile.x * SUBTILES_PER_TILE + w * SUBTILES_PER_TILE / 2,
            y: tile.y * SUBTILES_PER_TILE + h * SUBTILES_PER_TILE / 2,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrototypeDb {
    pub items: BTreeMap<String, ItemProto>,
    pub recipes: BTreeMap<String, RecipeProto>,
    pub entities: BTreeMap<String, EntityProto>,
}
