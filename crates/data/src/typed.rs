//! Converts raw `data.raw` into the simulation's typed, fixed-point [`PrototypeDb`].
//!
//! Prototype categories (which `type`s are items, entities, ...) come from the game's
//! `defines.prototypes`, so new types added by later game versions or DLC are picked up
//! without code changes. Default values follow the prototype API documentation.

use std::collections::BTreeMap;

use factorio_sim::Fixed;
use factorio_sim::map::{BoundingBox, Direction, SUBTILES_PER_TILE};
use factorio_sim::proto::*;

use crate::datastage::GameData;
use crate::raw::RawValue;
use crate::{Error, Result};

struct Names {
    items: BTreeMap<String, ItemId>,
    fluids: BTreeMap<String, FluidId>,
    recipes: BTreeMap<String, RecipeId>,
    entities: BTreeMap<String, EntityProtoId>,
    layers: BTreeMap<String, u32>,
}

fn sorted_names<'a>(it: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut v: Vec<String> = it.map(str::to_owned).collect();
    v.sort();
    v.dedup();
    v
}

fn fx(v: &RawValue) -> Option<Fixed> {
    v.as_f64().map(Fixed::from_f64_at_load)
}

fn fx_or(v: &RawValue, d: f64) -> Fixed {
    Fixed::from_f64_at_load(v.as_f64().unwrap_or(d))
}

fn subtiles(v: f64) -> i32 {
    (v * SUBTILES_PER_TILE as f64).round() as i32
}

/// `{x, y}` or `[x, y]` in tiles.
fn vector(v: &RawValue) -> [f64; 2] {
    [v.at(0).as_f64().or(v.get("x").as_f64()).unwrap_or(0.0), v.at(1).as_f64().or(v.get("y").as_f64()).unwrap_or(0.0)]
}

fn vector_subtiles(v: &RawValue) -> [i32; 2] {
    let [x, y] = vector(v);
    [subtiles(x), subtiles(y)]
}

fn vector_fixed(v: &RawValue) -> [Fixed; 2] {
    let [x, y] = vector(v);
    [Fixed::from_f64_at_load(x), Fixed::from_f64_at_load(y)]
}

fn bounding_box(b: &RawValue) -> BoundingBox {
    let lt = if b.get("left_top").is_nil() { b.at(0) } else { b.get("left_top") };
    let rb = if b.get("right_bottom").is_nil() { b.at(1) } else { b.get("right_bottom") };
    BoundingBox::new(vector_subtiles(lt), vector_subtiles(rb))
}

fn strings(v: &RawValue) -> Vec<String> {
    v.as_array().iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()
}

/// Parses a Factorio energy string such as `"4MJ"` or `"150kW"` into joules (or watts).
pub fn parse_energy(s: &str) -> std::result::Result<f64, String> {
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
    Ok(value * mult)
}

/// Energy (J) as fixed point.
fn energy(v: &RawValue) -> Option<Energy> {
    v.as_str().and_then(|s| parse_energy(s).ok()).map(Fixed::from_f64_at_load)
}

/// Power (W) as joules per tick.
fn power_per_tick(v: &RawValue) -> Option<Energy> {
    v.as_str().and_then(|s| parse_energy(s).ok()).map(|w| Fixed::from_f64_at_load(w / 60.0))
}

impl Names {
    fn mask(&self, v: &RawValue) -> CollisionMask {
        let mut layers = 0u64;
        if let Some(t) = v.get("layers").as_table() {
            for (name, on) in t {
                if on.as_bool() == Some(true)
                    && let Some(bit) = self.layers.get(name)
                {
                    layers |= 1 << bit;
                }
            }
        }
        CollisionMask {
            layers,
            not_colliding_with_itself: v.get("not_colliding_with_itself").as_bool().unwrap_or(false),
            colliding_with_tiles_only: v.get("colliding_with_tiles_only").as_bool().unwrap_or(false),
        }
    }

    fn layer_bits(&self, v: &RawValue) -> u64 {
        self.mask(v).layers
    }

    fn item_or_fluid(&self, kind: &str, name: &str) -> Option<ItemOrFluid> {
        if kind == "fluid" {
            self.fluids.get(name).map(|f| ItemOrFluid::Fluid(*f))
        } else {
            self.items.get(name).map(|i| ItemOrFluid::Item(*i))
        }
    }

    fn ingredients(&self, v: &RawValue) -> std::result::Result<Vec<Ingredient>, String> {
        v.as_array()
            .iter()
            .map(|i| {
                let name = i.get("name").as_str().ok_or("ingredient without name")?;
                let kind = i.get("type").as_str().unwrap_or("item");
                let what = self.item_or_fluid(kind, name).ok_or_else(|| format!("unknown {kind} {name}"))?;
                let amount = fx(i.get("amount")).ok_or("ingredient without amount")?;
                Ok(Ingredient { what, amount })
            })
            .collect()
    }

    fn products(&self, v: &RawValue) -> std::result::Result<Vec<Product>, String> {
        v.as_array()
            .iter()
            .map(|p| {
                let name = p.get("name").as_str().ok_or("product without name")?;
                let kind = p.get("type").as_str().unwrap_or("item");
                let what = self.item_or_fluid(kind, name).ok_or_else(|| format!("unknown {kind} {name}"))?;
                let (min, max) = match fx(p.get("amount")) {
                    Some(a) => (a, a),
                    None => (
                        fx(p.get("amount_min")).ok_or("product without amount")?,
                        fx(p.get("amount_max")).ok_or("product without amount_max")?,
                    ),
                };
                Ok(Product {
                    what,
                    amount_min: min,
                    amount_max: max,
                    probability: fx_or(p.get("probability"), 1.0),
                    temperature: fx(p.get("temperature")),
                })
            })
            .collect()
    }

    fn energy_source(&self, v: &RawValue, default_drain: Energy) -> EnergySource {
        match v.get("type").as_str() {
            Some("burner") => EnergySource::Burner {
                effectivity: fx_or(v.get("effectivity"), 1.0),
                fuel_inventory_size: v.get("fuel_inventory_size").as_i64().unwrap_or(1) as u32,
                fuel_categories: match strings(v.get("fuel_categories")) {
                    c if c.is_empty() => vec!["chemical".into()],
                    c => c,
                },
            },
            Some("electric") => EnergySource::Electric {
                drain: power_per_tick(v.get("drain")).unwrap_or(default_drain),
                buffer_capacity: energy(v.get("buffer_capacity")),
                output: v.get("usage_priority").as_str().is_some_and(|p| p.contains("output")),
            },
            Some("void") => EnergySource::Void,
            _ => EnergySource::Unsupported,
        }
    }

    fn fluid_box(&self, v: &RawValue) -> FluidBoxProto {
        let connections = v
            .get("pipe_connections")
            .as_array()
            .iter()
            .map(|c| PipeConnection {
                position: vector_subtiles(c.get("position")),
                direction: Direction(c.get("direction").as_i64().unwrap_or(0) as u8),
                flow: match c.get("flow_direction").as_str() {
                    Some("input") => FlowDirection::Input,
                    Some("output") => FlowDirection::Output,
                    _ => FlowDirection::InputOutput,
                },
                underground_max_distance: (c.get("connection_type").as_str() == Some("underground"))
                    .then(|| c.get("max_underground_distance").as_i64().unwrap_or(10) as u32),
            })
            .collect();
        FluidBoxProto {
            volume: fx(v.get("volume")).unwrap_or(Fixed::from_int(100)),
            filter: v.get("filter").as_str().and_then(|f| self.fluids.get(f).copied()),
            connections,
            minimum_temperature: fx(v.get("minimum_temperature")),
        }
    }
}

const ROTATABLE_TYPES: &[&str] = &[
    "transport-belt",
    "underground-belt",
    "splitter",
    "loader",
    "loader-1x1",
    "inserter",
    "mining-drill",
    "boiler",
    "generator",
    "offshore-pump",
    "pipe-to-ground",
    "pump",
    "assembling-machine",
    "storage-tank",
];

pub fn build_prototype_db(data: &GameData) -> Result<PrototypeDb> {
    let item_list: Vec<(String, String)> = {
        let mut v: Vec<_> = data.prototypes_in_category("item").map(|(k, n, _)| (n.to_owned(), k.to_owned())).collect();
        v.sort();
        v
    };
    let entity_list: Vec<(String, String)> = {
        let mut v: Vec<_> =
            data.prototypes_in_category("entity").map(|(k, n, _)| (n.to_owned(), k.to_owned())).collect();
        v.sort();
        v
    };
    let fluid_names = sorted_names(data.raw.get("fluid").as_table().into_iter().flatten().map(|(n, _)| n.as_str()));
    let recipe_names = sorted_names(data.raw.get("recipe").as_table().into_iter().flatten().map(|(n, _)| n.as_str()));
    let tile_names = sorted_names(data.raw.get("tile").as_table().into_iter().flatten().map(|(n, _)| n.as_str()));
    let layer_names =
        sorted_names(data.raw.get("collision-layer").as_table().into_iter().flatten().map(|(n, _)| n.as_str()));
    if layer_names.len() > 64 {
        return Err(Error::Mod(format!("{} collision layers; at most 64 are supported", layer_names.len())));
    }

    let names = Names {
        items: item_list.iter().enumerate().map(|(i, (n, _))| (n.clone(), ItemId(i as u16))).collect(),
        fluids: fluid_names.iter().enumerate().map(|(i, n)| (n.clone(), FluidId(i as u16))).collect(),
        recipes: recipe_names.iter().enumerate().map(|(i, n)| (n.clone(), RecipeId(i as u16))).collect(),
        entities: entity_list.iter().enumerate().map(|(i, (n, _))| (n.clone(), EntityProtoId(i as u16))).collect(),
        layers: layer_names.iter().enumerate().map(|(i, n)| (n.clone(), i as u32)).collect(),
    };

    // Factorio orders items by group, subgroup, then their own `order` string, then name.
    let order_key = |kind: &str, name: &str| {
        let p = data.prototype(kind, name);
        let subgroup = p.get("subgroup").as_str().unwrap_or("other");
        let sub = data.prototype("item-subgroup", subgroup);
        let group = sub.get("group").as_str().unwrap_or("");
        let s = |v: &RawValue| v.as_str().unwrap_or("").to_owned();
        (s(data.prototype("item-group", group).get("order")), s(sub.get("order")), s(p.get("order")), name.to_owned())
    };
    let mut by_order: Vec<usize> = (0..item_list.len()).collect();
    by_order.sort_by_cached_key(|i| order_key(&item_list[*i].1, &item_list[*i].0));
    let mut sort_index = vec![0u32; item_list.len()];
    for (rank, i) in by_order.into_iter().enumerate() {
        sort_index[i] = rank as u32;
    }

    let mut items = Vec::new();
    for (index, (name, kind)) in item_list.iter().enumerate() {
        let p = data.prototype(kind, name);
        let err = |m: &str| Error::Prototype { kind: kind.clone(), name: name.clone(), message: m.into() };
        let fuel = match (p.get("fuel_value").as_str(), p.get("fuel_category").as_str()) {
            (Some(v), Some(cat)) => {
                let value = parse_energy(v).map_err(|m| err(&m))?;
                (value > 0.0).then(|| Fuel {
                    category: cat.to_owned(),
                    value: Fixed::from_f64_at_load(value),
                    burnt_result: p.get("burnt_result").as_str().and_then(|b| names.items.get(b).copied()),
                })
            }
            _ => None,
        };
        items.push(ItemProto {
            name: name.clone(),
            kind: kind.clone(),
            stack_size: p.get("stack_size").as_i64().ok_or_else(|| err("missing stack_size"))? as u32,
            place_result: p.get("place_result").as_str().and_then(|e| names.entities.get(e).copied()),
            fuel,
            sort_index: sort_index[index],
            durability: (kind == "tool").then(|| fx_or(p.get("durability"), 1.0)),
        });
    }

    let fluids = fluid_names
        .iter()
        .map(|name| {
            let p = data.prototype("fluid", name);
            FluidProto {
                name: name.clone(),
                default_temperature: fx_or(p.get("default_temperature"), 15.0),
                heat_capacity: energy(p.get("heat_capacity")).unwrap_or(Fixed::from_int(1000)),
            }
        })
        .collect();

    let mut recipes = Vec::new();
    for name in &recipe_names {
        let p = data.prototype("recipe", name);
        let err = |m: String| Error::Prototype { kind: "recipe".into(), name: name.clone(), message: m };
        let results = names.products(p.get("results")).map_err(err)?;
        let main_product = match p.get("main_product").as_str() {
            Some("") => None,
            Some(m) => names.item_or_fluid("item", m).or_else(|| names.item_or_fluid("fluid", m)),
            None if results.len() == 1 => Some(results[0].what),
            None => None,
        };
        recipes.push(RecipeProto {
            name: name.clone(),
            category: p.get("category").as_str().unwrap_or("crafting").into(),
            energy_required: fx_or(p.get("energy_required"), 0.5),
            energy_required_ticks: Fixed::from_f64_at_load(p.get("energy_required").as_f64().unwrap_or(0.5) * 60.0),
            ingredients: names.ingredients(p.get("ingredients")).map_err(err)?,
            results,
            enabled: p.get("enabled").as_bool().unwrap_or(true),
            main_product,
            allow_as_intermediate: p.get("allow_as_intermediate").as_bool().unwrap_or(true),
            // Parameter recipes (for blueprint parametrisation) are never shown or used.
            hidden: p.get("hidden").as_bool().unwrap_or(false) || p.get("parameter").as_bool().unwrap_or(false),
        });
    }

    let mut entities = Vec::new();
    for (name, kind) in &entity_list {
        let p = data.prototype(kind, name);
        entities.push(entity_proto(&names, kind, name, p).map_err(|m| Error::Prototype {
            kind: kind.clone(),
            name: name.clone(),
            message: m,
        })?);
    }

    let tiles = tile_names
        .iter()
        .map(|name| {
            let p = data.prototype("tile", name);
            let c = p.get("map_color");
            let channel = |i: usize, k: &str| {
                let v = c.at(i).as_f64().or(c.get(k).as_f64()).unwrap_or(0.0);
                (if v > 1.0 { v } else { v * 255.0 }).round().clamp(0.0, 255.0) as u8
            };
            TileProto {
                name: name.clone(),
                collision_mask: names.mask(p.get("collision_mask")),
                walking_speed_modifier: fx_or(p.get("walking_speed_modifier"), 1.0),
                map_color: [channel(0, "r"), channel(1, "g"), channel(2, "b")],
                fluid: p.get("fluid").as_str().and_then(|f| names.fluids.get(f).copied()),
                layer: p.get("layer").as_i64().unwrap_or(0) as i32,
                autoplace: !p.get("autoplace").is_nil(),
            }
        })
        .collect();

    let mut db = PrototypeDb::new(items, fluids, recipes, entities, tiles, layer_names);
    db.set_technologies(technologies(data, &names)?);
    Ok(db)
}

fn technologies(data: &GameData, names: &Names) -> Result<Vec<TechnologyProto>> {
    let tech_names = sorted_names(data.raw.get("technology").as_table().into_iter().flatten().map(|(n, _)| n.as_str()));
    let ids: BTreeMap<&str, TechId> =
        tech_names.iter().enumerate().map(|(i, n)| (n.as_str(), TechId(i as u16))).collect();
    let mut out = Vec::new();
    for name in &tech_names {
        let p = data.prototype("technology", name);
        let err = |m: String| Error::Prototype { kind: "technology".into(), name: name.clone(), message: m };
        let item = |n: &str| names.items.get(n).copied().ok_or_else(|| err(format!("unknown item {n}")));
        let entity = |n: &str| names.entities.get(n).copied().ok_or_else(|| err(format!("unknown entity {n}")));

        let unit = match p.get("unit") {
            RawValue::Nil => None,
            u => {
                let count = match (u.get("count").as_f64(), u.get("count_formula").as_str()) {
                    (Some(c), _) => ResearchCount::Fixed(c.round() as u64),
                    (None, Some(f)) => ResearchCount::Formula(CountFormula::parse(f).map_err(err)?),
                    (None, None) => return Err(err("unit without count".into())),
                };
                let mut ingredients = Vec::new();
                for i in u.get("ingredients").as_array() {
                    let (n, amount) = match i.get("name").as_str() {
                        Some(n) => (n, i.get("amount").as_i64().unwrap_or(1)),
                        None => (i.at(0).as_str().unwrap_or(""), i.at(1).as_i64().unwrap_or(1)),
                    };
                    ingredients.push((item(n)?, amount as u32));
                }
                Some(ResearchUnit {
                    count,
                    ingredients,
                    time_ticks: (u.get("time").as_f64().unwrap_or(0.0) * 60.0).round() as u32,
                })
            }
        };

        let trigger = match p.get("research_trigger") {
            RawValue::Nil => None,
            t => Some(match t.get("type").as_str().unwrap_or("") {
                "craft-item" => {
                    let n = t.get("item").as_str().or(t.get("item").get("name").as_str()).unwrap_or("");
                    ResearchTrigger::CraftItem { item: item(n)?, count: t.get("count").as_i64().unwrap_or(1) as u32 }
                }
                "mine-entity" => {
                    ResearchTrigger::MineEntity { entity: entity(t.get("entity").as_str().unwrap_or(""))? }
                }
                "build-entity" => {
                    let n = t.get("entity").as_str().or(t.get("entity").get("name").as_str()).unwrap_or("");
                    ResearchTrigger::BuildEntity { entity: entity(n)? }
                }
                "craft-fluid" => ResearchTrigger::CraftFluid {
                    fluid: names
                        .fluids
                        .get(t.get("fluid").as_str().unwrap_or(""))
                        .copied()
                        .ok_or_else(|| err("unknown fluid".into()))?,
                    amount: fx_or(t.get("amount"), 0.0),
                },
                other => ResearchTrigger::Other(other.to_owned()),
            }),
        };

        let mut effects = Vec::new();
        for e in p.get("effects").as_array() {
            let kind = e.get("type").as_str().unwrap_or("");
            effects.push(match kind {
                "unlock-recipe" => {
                    let r = e.get("recipe").as_str().unwrap_or("");
                    TechEffect::UnlockRecipe(
                        names.recipes.get(r).copied().ok_or_else(|| err(format!("unknown recipe {r}")))?,
                    )
                }
                "give-item" => TechEffect::GiveItem {
                    item: item(e.get("item").as_str().unwrap_or(""))?,
                    count: e.get("count").as_i64().unwrap_or(1) as u32,
                },
                _ => {
                    let qualifier = ["ammo_category", "turret_id", "recipe", "entity", "item"]
                        .iter()
                        .find_map(|k| e.get(k).as_str())
                        .unwrap_or("")
                        .to_owned();
                    let modifier = match e.get("modifier") {
                        RawValue::Nil => Fixed::ONE,
                        m => match m.as_bool() {
                            Some(b) => Fixed::from_int(b as i64),
                            None => fx_or(m, 0.0),
                        },
                    };
                    TechEffect::Modifier { kind: kind.to_owned(), qualifier, modifier }
                }
            });
        }

        let mut prerequisites = Vec::new();
        for pre in strings(p.get("prerequisites")) {
            prerequisites.push(*ids.get(pre.as_str()).ok_or_else(|| err(format!("unknown prerequisite {pre}")))?);
        }
        let level = name.rsplit_once('-').and_then(|(_, n)| n.parse::<u32>().ok()).unwrap_or(1);
        let max_level = match p.get("max_level") {
            RawValue::Nil => Some(level),
            m if m.as_str() == Some("infinite") => None,
            m => Some(m.as_i64().unwrap_or(level as i64) as u32),
        };
        out.push(TechnologyProto {
            name: name.clone(),
            prerequisites,
            unit,
            trigger,
            effects,
            enabled: p.get("enabled").as_bool().unwrap_or(true),
            hidden: p.get("hidden").as_bool().unwrap_or(false),
            upgrade: p.get("upgrade").as_bool().unwrap_or(false),
            essential: p.get("essential").as_bool().unwrap_or(false),
            level,
            max_level,
            order: p.get("order").as_str().unwrap_or("").to_owned(),
        });
    }
    Ok(out)
}

fn entity_proto(names: &Names, kind: &str, name: &str, p: &RawValue) -> std::result::Result<EntityProto, String> {
    let collision_box = bounding_box(p.get("collision_box"));
    let size = |a: i32, b: i32| ((b - a + SUBTILES_PER_TILE - 1) / SUBTILES_PER_TILE).max(1);
    let tile_width = p
        .get("tile_width")
        .as_i64()
        .map(|v| v as i32)
        .unwrap_or(size(collision_box.left_top[0], collision_box.right_bottom[0]));
    let tile_height = p
        .get("tile_height")
        .as_i64()
        .map(|v| v as i32)
        .unwrap_or(size(collision_box.left_top[1], collision_box.right_bottom[1]));
    let flags = strings(p.get("flags"));

    let minable = match p.get("minable") {
        RawValue::Nil => None,
        m => {
            let results = if m.get("results").is_nil() {
                match m.get("result").as_str() {
                    Some(r) => vec![Product {
                        what: ItemOrFluid::Item(*names.items.get(r).ok_or_else(|| format!("unknown item {r}"))?),
                        amount_min: Fixed::from_int(m.get("count").as_i64().unwrap_or(1)),
                        amount_max: Fixed::from_int(m.get("count").as_i64().unwrap_or(1)),
                        probability: Fixed::ONE,
                        temperature: None,
                    }],
                    None => Vec::new(),
                }
            } else {
                names.products(m.get("results"))?
            };
            Some(Minable {
                mining_time: fx_or(m.get("mining_time"), 0.0),
                mining_ticks: Fixed::from_f64_at_load(m.get("mining_time").as_f64().unwrap_or(0.0) * 60.0),
                results,
                required_fluid: m.get("required_fluid").as_str().and_then(|f| names.fluids.get(f).copied()),
            })
        }
    };

    let tile_rules = p
        .get("tile_buildability_rules")
        .as_array()
        .iter()
        .map(|r| TileRule {
            area: bounding_box(r.get("area")),
            required: names.layer_bits(r.get("required_tiles")),
            colliding: names.layer_bits(r.get("colliding_tiles")),
        })
        .collect();

    let usage = |k: &str| power_per_tick(p.get(k)).unwrap_or(Fixed::ZERO);
    let data = match kind {
        "resource" => EntityData::Resource {
            infinite: p.get("infinite").as_bool().unwrap_or(false),
            category: p.get("category").as_str().unwrap_or("basic-solid").into(),
            stage_counts: p
                .get("stage_counts")
                .as_array()
                .iter()
                .filter_map(|v| v.as_i64())
                .map(|v| v as u32)
                .collect(),
            autoplace: !p.get("autoplace").is_nil(),
        },
        "character" => EntityData::Character {
            running_speed: fx_or(p.get("running_speed"), 0.15),
            mining_speed: fx_or(p.get("mining_speed"), 0.5),
            inventory_size: p.get("inventory_size").as_i64().unwrap_or(80) as u32,
            build_distance: fx_or(p.get("build_distance"), 10.0),
            reach_distance: fx_or(p.get("reach_distance"), 10.0),
            reach_resource_distance: fx_or(p.get("reach_resource_distance"), 2.7),
            item_pickup_distance: fx_or(p.get("item_pickup_distance"), 1.0),
            loot_pickup_distance: fx_or(p.get("loot_pickup_distance"), 2.0),
            mining_categories: strings(p.get("mining_categories")),
            crafting_categories: strings(p.get("crafting_categories")),
        },
        "container" | "logistic-container" => {
            EntityData::Container { inventory_size: p.get("inventory_size").as_i64().unwrap_or(0) as u32 }
        }
        "mining-drill" => EntityData::MiningDrill {
            mining_speed: fx_or(p.get("mining_speed"), 1.0),
            energy_usage: usage("energy_usage"),
            energy_source: names.energy_source(p.get("energy_source"), Fixed::ZERO),
            radius: subtiles(p.get("resource_searching_radius").as_f64().unwrap_or(0.49)),
            output_vector: vector_subtiles(p.get("vector_to_place_result")),
            resource_categories: strings(p.get("resource_categories")),
        },
        "furnace" | "assembling-machine" => {
            let energy_usage = usage("energy_usage");
            EntityData::CraftingMachine {
                furnace: kind == "furnace",
                crafting_speed: fx_or(p.get("crafting_speed"), 1.0),
                crafting_categories: strings(p.get("crafting_categories")),
                energy_usage,
                // Crafting machines default to a drain of 1/30 of their usage.
                energy_source: names.energy_source(p.get("energy_source"), energy_usage.div_int(30)),
                source_inventory_size: p.get("source_inventory_size").as_i64().map(|v| v as u32),
                result_inventory_size: p.get("result_inventory_size").as_i64().map(|v| v as u32),
                fixed_recipe: p.get("fixed_recipe").as_str().and_then(|r| names.recipes.get(r).copied()),
                fluid_boxes: p.get("fluid_boxes").as_array().iter().map(|b| names.fluid_box(b)).collect(),
            }
        }
        "inserter" => EntityData::Inserter {
            rotation_speed: fx_or(p.get("rotation_speed"), 0.01),
            extension_speed: fx_or(p.get("extension_speed"), 0.01),
            pickup_position: vector_fixed(p.get("pickup_position")),
            insert_position: vector_fixed(p.get("insert_position")),
            energy_per_movement: energy(p.get("energy_per_movement")).unwrap_or(Fixed::ZERO),
            energy_per_rotation: energy(p.get("energy_per_rotation")).unwrap_or(Fixed::ZERO),
            energy_source: names.energy_source(p.get("energy_source"), Fixed::ZERO),
        },
        "transport-belt" => EntityData::TransportBelt { speed: subtiles(p.get("speed").as_f64().unwrap_or(0.0)) },
        "underground-belt" => EntityData::UndergroundBelt {
            speed: subtiles(p.get("speed").as_f64().unwrap_or(0.0)),
            max_distance: p.get("max_distance").as_i64().unwrap_or(5) as u32,
        },
        "splitter" => EntityData::Splitter { speed: subtiles(p.get("speed").as_f64().unwrap_or(0.0)) },
        "electric-pole" => EntityData::ElectricPole {
            supply_area_distance: fx_or(p.get("supply_area_distance"), 0.0),
            maximum_wire_distance: fx_or(p.get("maximum_wire_distance"), 0.0),
        },
        "offshore-pump" => EntityData::OffshorePump {
            pumping_speed: fx_or(p.get("pumping_speed"), 0.0),
            fluid_source_offset: vector_subtiles(p.get("fluid_source_offset")),
            fluid_box: names.fluid_box(p.get("fluid_box")),
        },
        "boiler" => EntityData::Boiler {
            energy_consumption: usage("energy_consumption"),
            energy_source: names.energy_source(p.get("energy_source"), Fixed::ZERO),
            target_temperature: fx_or(p.get("target_temperature"), 165.0),
            fluid_box: names.fluid_box(p.get("fluid_box")),
            output_fluid_box: names.fluid_box(p.get("output_fluid_box")),
        },
        "generator" => EntityData::Generator {
            fluid_usage_per_tick: fx_or(p.get("fluid_usage_per_tick"), 0.0),
            maximum_temperature: fx_or(p.get("maximum_temperature"), 0.0),
            effectivity: fx_or(p.get("effectivity"), 1.0),
            fluid_box: names.fluid_box(p.get("fluid_box")),
        },
        "lab" => {
            let mut inputs = Vec::new();
            for i in strings(p.get("inputs")) {
                inputs.push(*names.items.get(&i).ok_or_else(|| format!("unknown lab input {i}"))?);
            }
            let energy_usage = usage("energy_usage");
            EntityData::Lab {
                inputs,
                researching_speed: fx_or(p.get("researching_speed"), 1.0),
                energy_usage,
                // Like crafting machines, labs drain 1/30 of their usage when idle.
                energy_source: names.energy_source(p.get("energy_source"), energy_usage.div_int(30)),
            }
        }
        "cliff" => {
            let mut orientations: Vec<CliffOrientation> = p
                .get("orientations")
                .as_table()
                .into_iter()
                .flatten()
                .map(|(name, o)| {
                    // The box may be rotated (third value, in turns); use its bounding box.
                    let b = o.get("collision_bounding_box");
                    let (lt, rb) = (vector(b.at(0)), vector(b.at(1)));
                    let turn = b.at(2).as_f64().unwrap_or(0.0) * std::f64::consts::TAU;
                    let (cx, cy) = ((lt[0] + rb[0]) / 2.0, (lt[1] + rb[1]) / 2.0);
                    let (hw, hh) = ((rb[0] - lt[0]) / 2.0, (rb[1] - lt[1]) / 2.0);
                    let ex = hw * turn.cos().abs() + hh * turn.sin().abs();
                    let ey = hw * turn.sin().abs() + hh * turn.cos().abs();
                    CliffOrientation {
                        name: name.clone(),
                        collision_box: BoundingBox::new(
                            [subtiles(cx - ex), subtiles(cy - ey)],
                            [subtiles(cx + ex), subtiles(cy + ey)],
                        ),
                        variations: o.get("pictures").as_array().len().clamp(1, 255) as u8,
                    }
                })
                .collect();
            orientations.sort_by(|a, b| a.name.cmp(&b.name));
            let grid = vector(p.get("grid_size"));
            EntityData::Cliff {
                orientations,
                grid_size: [subtiles(grid[0].max(1.0)), subtiles(grid[1].max(1.0))],
                grid_offset: vector_subtiles(p.get("grid_offset")),
            }
        }
        "pipe" | "pipe-to-ground" => EntityData::Pipe { fluid_box: names.fluid_box(p.get("fluid_box")) },
        _ => EntityData::Other,
    };

    Ok(EntityProto {
        name: name.to_owned(),
        kind: kind.to_owned(),
        collision_box,
        collision_mask: names.mask(p.get("collision_mask")),
        tile_width,
        tile_height,
        minable,
        rotatable: ROTATABLE_TYPES.contains(&kind) && !flags.iter().any(|f| f == "not-rotatable"),
        tile_rules,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn energy_strings() {
        assert_eq!(parse_energy("4MJ"), Ok(4_000_000.0));
        assert_eq!(parse_energy("1.5kW"), Ok(1500.0));
        assert_eq!(parse_energy("100J"), Ok(100.0));
    }
}
