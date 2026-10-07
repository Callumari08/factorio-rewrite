//! Typed, simulation-ready prototypes.
//!
//! These are produced from Factorio's Lua data stage by the `factorio-data` crate, with
//! every float converted to fixed point at load time. Everything is keyed by name in a
//! `BTreeMap` so iteration order is identical on every machine, and every type is open
//! for extension by mods (no enum of base-game names anywhere).

use std::collections::BTreeMap;

use crate::fixed::Fixed;

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrototypeDb {
    pub items: BTreeMap<String, ItemProto>,
    pub recipes: BTreeMap<String, RecipeProto>,
    pub entities: BTreeMap<String, EntityProto>,
}
