//! Converts raw `data.raw` into the simulation's typed, fixed-point [`PrototypeDb`].
//!
//! Prototype categories (which `type`s are items, entities, ...) come from the game's
//! `defines.prototypes`, so new types added by later game versions or DLC are picked up
//! without code changes.

use factorio_sim::Fixed;
use factorio_sim::map::SUBTILES_PER_TILE;
use factorio_sim::proto::{EntityProto, ItemProto, PrototypeDb, RecipeIngredient, RecipeProto};

use crate::datastage::GameData;
use crate::raw::RawValue;
use crate::{Error, Result};

pub fn build_prototype_db(data: &GameData) -> Result<PrototypeDb> {
    let mut db = PrototypeDb::default();

    for (kind, name, p) in data.prototypes_in_category("item") {
        let err = |m: &str| Error::Prototype { kind: kind.into(), name: name.into(), message: m.into() };
        let stack_size = p.get("stack_size").as_i64().ok_or_else(|| err("missing stack_size"))?;
        let fuel = p.get("fuel_value").as_str().map(parse_energy).transpose().map_err(|m| err(&m))?;
        db.items.insert(
            name.into(),
            ItemProto {
                name: name.into(),
                kind: kind.into(),
                stack_size: stack_size as u32,
                place_result: p.get("place_result").as_str().map(str::to_owned),
                fuel_value_joules: fuel,
            },
        );
    }

    for (kind, name, p) in data.prototypes_in_category("recipe") {
        let err = |m: &str| Error::Prototype { kind: kind.into(), name: name.into(), message: m.into() };
        let energy = p.get("energy_required").as_f64().unwrap_or(0.5);
        db.recipes.insert(
            name.into(),
            RecipeProto {
                name: name.into(),
                category: p.get("category").as_str().unwrap_or("crafting").into(),
                energy_required_ticks: (energy * 60.0).round() as u32,
                ingredients: products(p.get("ingredients")).map_err(|m| err(&m))?,
                results: products(p.get("results")).map_err(|m| err(&m))?,
                enabled: p.get("enabled").as_bool().unwrap_or(true),
            },
        );
    }

    for (kind, name, p) in data.prototypes_in_category("entity") {
        db.entities.insert(
            name.into(),
            EntityProto {
                name: name.into(),
                kind: kind.into(),
                collision_box: bounding_box(p.get("collision_box")),
                belt_speed: (kind == "transport-belt" || kind == "underground-belt" || kind == "splitter")
                    .then(|| p.get("speed").as_f64().map(Fixed::from_f64_at_load))
                    .flatten(),
            },
        );
    }

    Ok(db)
}

fn products(list: &RawValue) -> std::result::Result<Vec<RecipeIngredient>, String> {
    list.as_array()
        .iter()
        .map(|i| {
            let name = i.get("name").as_str().ok_or("ingredient without name")?;
            let amount = match i.get("amount").as_f64() {
                Some(a) => a,
                None => {
                    let (lo, hi) = (i.get("amount_min").as_f64(), i.get("amount_max").as_f64());
                    (lo.ok_or("ingredient without amount")? + hi.ok_or("ingredient without amount_max")?) / 2.0
                }
            };
            Ok(RecipeIngredient {
                kind: i.get("type").as_str().unwrap_or("item").into(),
                name: name.into(),
                amount: Fixed::from_f64_at_load(amount),
            })
        })
        .collect()
}

/// `{{-0.4, -0.4}, {0.4, 0.4}}` (or `{left_top = ..., right_bottom = ...}`) in 1/256 tiles.
fn bounding_box(b: &RawValue) -> [[i32; 2]; 2] {
    let corner = |c: &RawValue| -> [i32; 2] {
        let x = c.at(0).as_f64().or(c.get("x").as_f64()).unwrap_or(0.0);
        let y = c.at(1).as_f64().or(c.get("y").as_f64()).unwrap_or(0.0);
        let s = SUBTILES_PER_TILE as f64;
        [(x * s).round() as i32, (y * s).round() as i32]
    };
    let lt = if b.get("left_top").is_nil() { b.at(0) } else { b.get("left_top") };
    let rb = if b.get("right_bottom").is_nil() { b.at(1) } else { b.get("right_bottom") };
    [corner(lt), corner(rb)]
}

/// Parses a Factorio energy string such as `"4MJ"` or `"150kW"` into joules (or watts).
pub fn parse_energy(s: &str) -> std::result::Result<i64, String> {
    let s = s.trim();
    let unit_start = s.find(|c: char| c.is_ascii_alphabetic()).ok_or_else(|| format!("bad energy '{s}'"))?;
    let (num, unit) = s.split_at(unit_start);
    let value: f64 = num.parse().map_err(|_| format!("bad energy '{s}'"))?;
    let (prefix, base) = unit.split_at(unit.len() - 1);
    if base != "J" && base != "W" {
        return Err(format!("bad energy unit '{s}'"));
    }
    let mult = match prefix {
        "" => 1.0,
        "k" | "K" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        "E" => 1e18,
        _ => return Err(format!("bad energy prefix '{s}'")),
    };
    Ok((value * mult).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn energy_strings() {
        assert_eq!(parse_energy("4MJ"), Ok(4_000_000));
        assert_eq!(parse_energy("1.5kW"), Ok(1500));
        assert_eq!(parse_energy("100J"), Ok(100));
    }
}
