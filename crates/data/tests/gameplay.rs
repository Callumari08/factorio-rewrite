//! Gameplay tests against the real Factorio data, comparing with known game values.
//!
//! These load the prototypes from your Factorio install. If no install is found they are
//! skipped (they print a note and pass), so `cargo test` still works without the game.

use std::sync::{Arc, OnceLock};

use factorio_sim::Fixed;
use factorio_sim::input::{InputAction, PlayerInput};
use factorio_sim::map::{Direction, MapPosition, TilePosition};
use factorio_sim::proto::{ItemId, PrototypeDb};
use factorio_sim::surface::{MapGenSettings, ResourceTile};
use factorio_sim::world::{EntityId, EntityState, Simulation};

fn data() -> Option<&'static (Arc<PrototypeDb>, MapGenSettings)> {
    static DATA: OnceLock<Option<(Arc<PrototypeDb>, MapGenSettings)>> = OnceLock::new();
    DATA.get_or_init(|| {
        let config = factorio_data::Config::load().ok()?;
        let data = match factorio_data::load_game_data(&config) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("skipping gameplay tests: {e}");
                return None;
            }
        };
        let db = factorio_data::typed::build_prototype_db(&data).expect("prototypes");
        let mapgen = factorio_data::mapgen::default_mapgen(&db, 1234);
        Some((Arc::new(db), mapgen))
    })
    .as_ref()
}

macro_rules! game {
    () => {
        match data() {
            Some(d) => d,
            None => return,
        }
    };
}

/// A world with flat grass and no resources around the origin, and player 0 joined.
fn flat_world(d: &(Arc<PrototypeDb>, MapGenSettings)) -> Simulation {
    let mut sim = Simulation::new(d.0.clone(), d.1.clone());
    let grass = d.0.tile_id("grass-1").unwrap();
    for y in -60..60 {
        for x in -60..60 {
            let t = TilePosition::new(x, y);
            sim.surface.set_tile(t, grass);
            sim.surface.set_resource(t, None);
        }
    }
    sim.step(&[PlayerInput::new(0, InputAction::JoinGame)]);
    sim
}

fn item(sim: &Simulation, name: &str) -> ItemId {
    sim.prototypes().item_id(name).unwrap_or_else(|| panic!("no item {name}"))
}

fn place(sim: &mut Simulation, name: &str, x: i32, y: i32, dir: Direction) -> EntityId {
    let proto = sim.prototypes().entity_id(name).unwrap();
    let pos = sim.prototypes().entity(proto).position_for_tile(TilePosition::new(x, y), dir);
    sim.place_entity(proto, pos, dir).unwrap_or_else(|e| panic!("cannot place {name} at {x},{y}: {e:?}"))
}

fn ore(sim: &mut Simulation, name: &str, x: i32, y: i32, amount: u32) {
    let proto = sim.prototypes().entity_id(name).unwrap();
    sim.surface.set_resource(TilePosition::new(x, y), Some(ResourceTile { proto, amount }));
}

fn inventory_count(sim: &Simulation, name: &str) -> u32 {
    let i = item(sim, name);
    sim.player(0).unwrap().character.as_ref().unwrap().inventory.count(i)
}

fn input(sim: &mut Simulation, action: InputAction) {
    sim.step(&[PlayerInput::new(0, action)]);
}

fn give(sim: &mut Simulation, name: &str, count: u32) {
    let i = item(sim, name);
    input(sim, InputAction::CheatItems { item: i, count });
}

fn container_count(sim: &Simulation, id: EntityId, name: &str) -> u32 {
    let i = item(sim, name);
    match &sim.entity(id).unwrap().state {
        EntityState::Container(inv) => inv.count(i),
        EntityState::Crafter(c) => c.output.count(i),
        other => panic!("not a container: {other:?}"),
    }
}

fn insert(sim: &mut Simulation, id: EntityId, name: &str, count: u32) {
    let i = item(sim, name);
    let n = sim.insert_into_entity(id, i, count, factorio_sim::world::InsertSource::Player);
    assert_eq!(n, count, "could not insert {count} {name}");
}

/// Steps until `f` holds, returning how many ticks it took.
fn ticks_until(sim: &mut Simulation, max: u32, mut f: impl FnMut(&Simulation) -> bool) -> u32 {
    for t in 1..=max {
        sim.step(&[]);
        if f(sim) {
            return t;
        }
    }
    panic!("condition not met within {max} ticks");
}

#[test]
fn hand_mining_iron_ore_takes_two_seconds() {
    let d = game!();
    let mut sim = flat_world(d);
    ore(&mut sim, "iron-ore", 1, 0, 50);
    // The mining input is applied at the start of a tick, which also counts as progress.
    input(&mut sim, InputAction::SetMining(Some(MapPosition::tile_center(TilePosition::new(1, 0)))));
    let t = 1 + ticks_until(&mut sim, 1000, |s| inventory_count(s, "iron-ore") == 1);
    // mining_time 1 / character mining_speed 0.5 = 2 s.
    assert_eq!(t, 120);
    let t2 = ticks_until(&mut sim, 1000, |s| inventory_count(s, "iron-ore") == 2);
    assert_eq!(t2, 120);
    let left = sim.surface.resource(TilePosition::new(1, 0)).unwrap().amount;
    assert_eq!(left, 48);
}

#[test]
fn hand_mining_a_chest_returns_it() {
    let d = game!();
    let mut sim = flat_world(d);
    let chest = place(&mut sim, "wooden-chest", 2, 0, Direction::NORTH);
    insert(&mut sim, chest, "iron-plate", 7);
    input(&mut sim, InputAction::SetMining(Some(MapPosition::tile_center(TilePosition::new(2, 0)))));
    let t = 1 + ticks_until(&mut sim, 100, |s| inventory_count(s, "wooden-chest") == 1);
    // mining_time 0.1 / 0.5 = 0.2 s = 12 ticks.
    assert_eq!(t, 12);
    assert_eq!(inventory_count(&sim, "iron-plate"), 7);
}

#[test]
fn hand_crafting_uses_recipe_time() {
    let d = game!();
    let mut sim = flat_world(d);
    give(&mut sim, "iron-plate", 10);
    let gear = sim.prototypes().recipe_id("iron-gear-wheel").unwrap();
    input(&mut sim, InputAction::Craft { recipe: gear, count: 5 });
    assert_eq!(inventory_count(&sim, "iron-plate"), 0, "ingredients are taken when queued");
    let t = 1 + ticks_until(&mut sim, 1000, |s| inventory_count(s, "iron-gear-wheel") == 5);
    // 0.5 s per gear.
    assert_eq!(t, 150);
}

#[test]
fn hand_crafting_makes_intermediates() {
    let d = game!();
    let mut sim = flat_world(d);
    give(&mut sim, "iron-plate", 3);
    let belt = sim.prototypes().recipe_id("transport-belt").unwrap();
    input(&mut sim, InputAction::Craft { recipe: belt, count: 1 });
    let t = 1 + ticks_until(&mut sim, 1000, |s| inventory_count(s, "transport-belt") == 2);
    // One gear (0.5 s) then the belts (0.5 s).
    assert_eq!(t, 60);
    assert_eq!(inventory_count(&sim, "iron-gear-wheel"), 0);
    assert_eq!(inventory_count(&sim, "iron-plate"), 0);
}

#[test]
fn cancelling_a_craft_refunds_ingredients() {
    let d = game!();
    let mut sim = flat_world(d);
    give(&mut sim, "iron-plate", 10);
    let gear = sim.prototypes().recipe_id("iron-gear-wheel").unwrap();
    input(&mut sim, InputAction::Craft { recipe: gear, count: 5 });
    for _ in 0..40 {
        sim.step(&[]);
    }
    input(&mut sim, InputAction::CancelCraft { index: 0 });
    assert_eq!(inventory_count(&sim, "iron-gear-wheel"), 1);
    assert_eq!(inventory_count(&sim, "iron-plate"), 8);
}

#[test]
fn burner_drill_mines_a_quarter_ore_per_second() {
    let d = game!();
    let mut sim = flat_world(d);
    // A burner drill facing north covers the 2x2 tiles at (10..12, 10..12) and drops at
    // (-0.5, -1.3) from its centre, i.e. into the tile (10, 8).
    for (x, y) in [(10, 10), (11, 10), (10, 11), (11, 11)] {
        ore(&mut sim, "iron-ore", x, y, 1000);
    }
    let drill = place(&mut sim, "burner-mining-drill", 10, 10, Direction::NORTH);
    let chest = place(&mut sim, "wooden-chest", 10, 9, Direction::NORTH);
    insert(&mut sim, drill, "coal", 5);
    let t = ticks_until(&mut sim, 10_000, |s| container_count(s, chest, "iron-ore") == 1);
    // mining_time 1 / mining_speed 0.25 = 4 s per ore.
    assert_eq!(t, 240);
    let t = ticks_until(&mut sim, 10_000, |s| container_count(s, chest, "iron-ore") == 11);
    assert_eq!(t, 2400);
}

#[test]
fn burner_drill_burns_coal_at_150kw() {
    let d = game!();
    let mut sim = flat_world(d);
    for (x, y) in [(10, 10), (11, 10), (10, 11), (11, 11)] {
        ore(&mut sim, "iron-ore", x, y, 10_000);
    }
    let drill = place(&mut sim, "burner-mining-drill", 10, 10, Direction::NORTH);
    let chest = place(&mut sim, "iron-chest", 10, 9, Direction::NORTH);
    insert(&mut sim, drill, "coal", 1);
    // 4 MJ / 150 kW = 26.67 s = 1600 ticks of work, i.e. 6 ores and most of a 7th.
    for _ in 0..3000 {
        sim.step(&[]);
    }
    assert_eq!(container_count(&sim, chest, "iron-ore"), 6);
}

#[test]
fn stone_furnace_smelts_iron_in_3_2_seconds() {
    let d = game!();
    let mut sim = flat_world(d);
    let furnace = place(&mut sim, "stone-furnace", 10, 10, Direction::NORTH);
    insert(&mut sim, furnace, "iron-ore", 10);
    insert(&mut sim, furnace, "coal", 2);
    let t = ticks_until(&mut sim, 10_000, |s| container_count(s, furnace, "iron-plate") == 1);
    assert_eq!(t, 192);
    let t = ticks_until(&mut sim, 10_000, |s| container_count(s, furnace, "iron-plate") == 6);
    assert_eq!(t, 5 * 192);
}

/// Feeds both lanes at the start of a belt line as fast as possible and counts items
/// leaving the end.
fn belt_throughput(sim: &mut Simulation, belt: &str, length: i32, ticks: u32) -> u32 {
    let plate = item(sim, "iron-plate");
    let ids: Vec<EntityId> = (0..length).map(|x| place(sim, belt, x - 20, 20, Direction::EAST)).collect();
    let (first, last) = (ids[0], *ids.last().unwrap());
    let mut arrived = 0;
    for t in 0..ticks + 600 {
        for lane in 0..2 {
            sim.belts.get_mut(first).unwrap().try_insert(lane, 0, plate);
        }
        sim.step(&[]);
        let b = sim.belts.get_mut(last).unwrap();
        for lane in 0..2 {
            // Take items off halfway along the last belt, before they slow down at its end.
            if b.lanes[lane].items.last().is_some_and(|i| i.pos >= 128) {
                b.lanes[lane].items.pop();
                if t >= 600 {
                    arrived += 1;
                }
            }
        }
    }
    arrived
}

#[test]
fn transport_belt_moves_15_items_per_second() {
    let d = game!();
    let mut sim = flat_world(d);
    let n = belt_throughput(&mut sim, "transport-belt", 10, 600);
    assert!((149..=151).contains(&n), "expected ~150 items in 10 s, got {n}");
}

#[test]
fn compressed_belt_stays_compressed_regardless_of_build_order() {
    let d = game!();
    let mut sim = flat_world(d);
    let plate = item(&sim, "iron-plate");
    // Build the line back to front so entity ids run against the item flow.
    let mut ids: Vec<EntityId> =
        (0..8).rev().map(|x| place(&mut sim, "transport-belt", x, 30, Direction::EAST)).collect();
    ids.reverse();
    for _ in 0..1200 {
        sim.belts.get_mut(ids[0]).unwrap().try_insert(0, 0, plate);
        sim.step(&[]);
    }
    // The line is backed up; every belt but the first should hold 4 items on lane 0.
    for id in &ids[1..] {
        assert_eq!(sim.belts.get(*id).unwrap().lanes[0].items.len(), 4);
    }
}

#[test]
fn burner_inserter_moves_0_79_items_per_second() {
    let d = game!();
    let mut sim = flat_world(d);
    let from = place(&mut sim, "iron-chest", 10, 10, Direction::NORTH);
    // North-facing inserter picks up from the north (y-1) and drops to the south (y+1).
    let ins = place(&mut sim, "burner-inserter", 10, 11, Direction::NORTH);
    let to = place(&mut sim, "iron-chest", 10, 12, Direction::NORTH);
    insert(&mut sim, from, "iron-plate", 400);
    insert(&mut sim, ins, "coal", 5);
    // Warm up one cycle, then count for 60 s.
    ticks_until(&mut sim, 1000, |s| container_count(s, to, "iron-plate") == 1);
    for _ in 0..3600 {
        sim.step(&[]);
    }
    let moved = container_count(&sim, to, "iron-plate") - 1;
    eprintln!("burner inserter moved {moved}");
    // 76 ticks per swing: 3600 / 76 = 47.4.
    assert!((47..=48).contains(&moved), "moved {moved}");
}

#[test]
fn burner_inserter_fuels_itself_from_coal_it_moves() {
    let d = game!();
    let mut sim = flat_world(d);
    let from = place(&mut sim, "iron-chest", 10, 10, Direction::NORTH);
    place(&mut sim, "burner-inserter", 10, 11, Direction::NORTH);
    let to = place(&mut sim, "iron-chest", 10, 12, Direction::NORTH);
    insert(&mut sim, from, "coal", 10);
    for _ in 0..2000 {
        sim.step(&[]);
    }
    assert!(container_count(&sim, to, "coal") > 0);
}

#[test]
fn inserter_feeds_furnace_within_insertion_limit() {
    let d = game!();
    let mut sim = flat_world(d);
    let from = place(&mut sim, "iron-chest", 10, 10, Direction::NORTH);
    let ins = place(&mut sim, "burner-inserter", 10, 11, Direction::NORTH);
    let furnace = place(&mut sim, "stone-furnace", 10, 12, Direction::NORTH);
    insert(&mut sim, from, "iron-ore", 50);
    insert(&mut sim, ins, "coal", 5);
    // No fuel in the furnace: it never smelts, so the inserter stops at the limit of 2.
    for _ in 0..3000 {
        sim.step(&[]);
    }
    let ore = item(&sim, "iron-ore");
    let EntityState::Crafter(c) = &sim.entity(furnace).unwrap().state else { panic!() };
    assert_eq!(c.input.count(ore), 2);
}

#[test]
fn drill_outputs_onto_belt() {
    let d = game!();
    let mut sim = flat_world(d);
    for (x, y) in [(10, 10), (11, 10), (10, 11), (11, 11)] {
        ore(&mut sim, "iron-ore", x, y, 1000);
    }
    let drill = place(&mut sim, "burner-mining-drill", 10, 10, Direction::NORTH);
    let belt = place(&mut sim, "transport-belt", 10, 9, Direction::WEST);
    insert(&mut sim, drill, "coal", 5);
    for _ in 0..241 {
        sim.step(&[]);
    }
    assert_eq!(sim.belts.get(belt).unwrap().item_count(), 1);
}

#[test]
fn steam_power_runs_an_assembler() {
    let d = game!();
    let mut sim = flat_world(d);
    let water = sim.prototypes().tile_id("water").unwrap();
    for y in 0..6 {
        for x in -10..10 {
            sim.surface.set_tile(TilePosition::new(x, y - 10), water);
        }
    }
    // Offshore pump on the shore taking water from the north and pumping south into the
    // boiler's water inlet; the boiler's steam outlet faces east into the steam engine.
    let pump = place(&mut sim, "offshore-pump", -1, -4, Direction::NORTH);
    assert!(matches!(sim.entity(pump).unwrap().state, EntityState::Fluid(_)));
    let boiler = place(&mut sim, "boiler", -1, -3, Direction::EAST);
    place(&mut sim, "steam-engine", 1, -3, Direction::EAST);
    insert(&mut sim, boiler, "coal", 20);
    place(&mut sim, "small-electric-pole", 1, 0, Direction::NORTH);
    let asm = place(&mut sim, "assembling-machine-1", 2, 0, Direction::NORTH);
    let gear = sim.prototypes().recipe_id("iron-gear-wheel").unwrap();
    give(&mut sim, "iron-plate", 0);
    // Put the assembler in reach of the character and set its recipe.
    input(
        &mut sim,
        InputAction::SetRecipe { position: MapPosition::tile_center(TilePosition::new(3, 1)), recipe: Some(gear) },
    );
    insert(&mut sim, asm, "iron-plate", 100);
    let t = ticks_until(&mut sim, 2000, |s| container_count(s, asm, "iron-gear-wheel") == 1);
    assert!(t < 600, "first gear after {t} ticks");
    let t = ticks_until(&mut sim, 2000, |s| container_count(s, asm, "iron-gear-wheel") == 11);
    // Crafting speed 0.5: 1 gear per second when fully powered.
    assert_eq!(t, 600);
    let net = &sim.power.electric_networks[0];
    assert!(net.satisfaction() == Fixed::ONE);
}

#[test]
fn identical_inputs_give_identical_checksums() {
    let d = game!();
    let run = || {
        let mut sim = flat_world(d);
        for (x, y) in [(10, 10), (11, 10), (10, 11), (11, 11)] {
            ore(&mut sim, "iron-ore", x, y, 1000);
        }
        let drill = place(&mut sim, "burner-mining-drill", 10, 10, Direction::NORTH);
        for x in 3..10 {
            place(&mut sim, "transport-belt", x, 9, Direction::WEST);
        }
        place(&mut sim, "transport-belt", 10, 9, Direction::WEST);
        insert(&mut sim, drill, "coal", 5);
        input(&mut sim, InputAction::SetWalking(Some(Direction(6))));
        for _ in 0..2000 {
            sim.step(&[]);
        }
        sim.checksum()
    };
    assert_eq!(run(), run());
}
