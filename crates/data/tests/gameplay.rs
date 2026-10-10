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

fn belt_items(sim: &Simulation, id: EntityId) -> usize {
    sim.belts.get(id).unwrap().item_count()
}

#[test]
fn curve_lanes_hold_fewer_inner_items() {
    let d = game!();
    let mut sim = flat_world(d);
    let plate = item(&sim, "iron-plate");
    // East along y=20, then turning north at x=5: items heading east turn north, a left
    // turn, so the corner's left lane is the inner one.
    let a = place(&mut sim, "transport-belt", 4, 20, Direction::EAST);
    let corner = place(&mut sim, "transport-belt", 5, 20, Direction::NORTH);
    sim.step(&[]);
    let b = sim.belts.get(corner).unwrap();
    assert_eq!(b.shape, factorio_sim::belt::BeltShape::CurveLeft);
    assert_eq!(b.lane_len(0), 106);
    assert_eq!(b.lane_len(1), 295);
    for _ in 0..600 {
        for lane in 0..2 {
            sim.belts.get_mut(a).unwrap().try_insert(lane, 0, plate);
        }
        sim.step(&[]);
    }
    let b = sim.belts.get(corner).unwrap();
    // Dead-ended curve: inner lane fits 2 items (0..106 with 64 spacing), outer 5.
    assert_eq!(b.lanes[0].items.len(), 2);
    assert_eq!(b.lanes[1].items.len(), 5);
}

#[test]
fn sideloading_fills_only_the_near_lane() {
    let d = game!();
    let mut sim = flat_world(d);
    let plate = item(&sim, "iron-plate");
    let main: Vec<EntityId> = (0..4).map(|y| place(&mut sim, "transport-belt", 10, 20 - y, Direction::NORTH)).collect();
    // A belt feeding into the side of the second main belt from the west, plus a belt
    // behind the main line so the main belt stays straight.
    let side = place(&mut sim, "transport-belt", 9, 19, Direction::EAST);
    for _ in 0..600 {
        for lane in 0..2 {
            sim.belts.get_mut(side).unwrap().try_insert(lane, 0, plate);
        }
        sim.step(&[]);
    }
    let top = sim.belts.get(main[3]).unwrap();
    // Facing north, the west side is the left lane.
    assert!(top.lanes[0].items.len() >= 3);
    assert_eq!(top.lanes[1].items.len(), 0);
}

#[test]
fn underground_belt_carries_items_across_a_gap() {
    let d = game!();
    let mut sim = flat_world(d);
    let plate = item(&sim, "iron-plate");
    let feed = place(&mut sim, "transport-belt", 0, 25, Direction::EAST);
    let entrance = place(&mut sim, "underground-belt", 1, 25, Direction::EAST);
    // Something in the way that the belt passes under.
    place(&mut sim, "wooden-chest", 2, 25, Direction::NORTH);
    let exit = place(&mut sim, "underground-belt", 5, 25, Direction::EAST);
    let out = place(&mut sim, "transport-belt", 6, 25, Direction::EAST);
    assert_eq!(sim.belts.get(entrance).unwrap().kind, factorio_sim::belt::BeltKind::UndergroundInput);
    assert_eq!(sim.belts.get(exit).unwrap().kind, factorio_sim::belt::BeltKind::UndergroundOutput);
    sim.belts.get_mut(feed).unwrap().try_insert(0, 0, plate);
    for _ in 0..600 {
        sim.step(&[]);
    }
    assert_eq!(belt_items(&sim, out), 1);
}

#[test]
fn splitter_alternates_between_outputs() {
    let d = game!();
    let mut sim = flat_world(d);
    let plate = item(&sim, "iron-plate");
    // Splitter facing north at y=30 covering x=10 and x=11; one input belt below x=10,
    // output belts above both halves.
    let feed = place(&mut sim, "transport-belt", 10, 31, Direction::NORTH);
    let proto = sim.prototypes().entity_id("splitter").unwrap();
    let pos = factorio_sim::map::MapPosition::new(11 * 256, 30 * 256 + 128);
    sim.place_entity(proto, pos, Direction::NORTH).unwrap();
    let left = place(&mut sim, "transport-belt", 10, 29, Direction::NORTH);
    let right = place(&mut sim, "transport-belt", 11, 29, Direction::NORTH);
    let lefts: Vec<EntityId> =
        (1..6).map(|i| place(&mut sim, "transport-belt", 10, 29 - i, Direction::NORTH)).collect();
    let rights: Vec<EntityId> =
        (1..6).map(|i| place(&mut sim, "transport-belt", 11, 29 - i, Direction::NORTH)).collect();
    let mut sent = 0;
    for _ in 0..1200 {
        if sent < 20 && sim.belts.get_mut(feed).unwrap().try_insert(0, 0, plate) {
            sent += 1;
        }
        sim.step(&[]);
    }
    let count = |ids: &[EntityId], first: EntityId, sim: &Simulation| {
        belt_items(sim, first) + ids.iter().map(|i| belt_items(sim, *i)).sum::<usize>()
    };
    let (l, r) = (count(&lefts, left, &sim), count(&rights, right, &sim));
    assert_eq!(l + r, 20);
    assert_eq!(l, 10);
    assert_eq!(r, 10);
}

/// Steam power for a block of consumers; returns the boiler entity.
fn steam_power(sim: &mut Simulation, engines: i32) -> EntityId {
    let water = sim.prototypes().tile_id("water").unwrap();
    for y in 0..6 {
        for x in -10..10 {
            sim.surface.set_tile(TilePosition::new(x, y - 10), water);
        }
    }
    place(sim, "offshore-pump", -1, -4, Direction::NORTH);
    let boiler = place(sim, "boiler", -1, -3, Direction::EAST);
    for i in 0..engines {
        place(sim, "steam-engine", 1 + 5 * i, -3, Direction::EAST);
    }
    insert(sim, boiler, "coal", 50);
    boiler
}

#[test]
fn electric_inserter_moves_0_86_items_per_second() {
    let d = game!();
    let mut sim = flat_world(d);
    steam_power(&mut sim, 1);
    place(&mut sim, "small-electric-pole", 1, 0, Direction::NORTH);
    let from = place(&mut sim, "iron-chest", 2, 0, Direction::NORTH);
    place(&mut sim, "inserter", 2, 1, Direction::NORTH);
    let to = place(&mut sim, "iron-chest", 2, 2, Direction::NORTH);
    insert(&mut sim, from, "iron-plate", 400);
    ticks_until(&mut sim, 2000, |s| container_count(s, to, "iron-plate") == 1);
    for _ in 0..3600 {
        sim.step(&[]);
    }
    let moved = container_count(&sim, to, "iron-plate") - 1;
    // 70 ticks per swing: 3600 / 70 = 51.4.
    assert!((51..=52).contains(&moved), "moved {moved}");
}

#[test]
fn inserter_stack_size_bonus_moves_two_per_swing() {
    let d = game!();
    let mut sim = flat_world(d);
    // Inserter capacity bonus 2 gives normal inserters +1 hand size.
    let tech = sim.prototypes().technology_id("inserter-capacity-bonus-2").unwrap();
    sim.finish_research(tech);
    steam_power(&mut sim, 1);
    place(&mut sim, "small-electric-pole", 1, 0, Direction::NORTH);
    let from = place(&mut sim, "iron-chest", 2, 0, Direction::NORTH);
    place(&mut sim, "inserter", 2, 1, Direction::NORTH);
    let to = place(&mut sim, "iron-chest", 2, 2, Direction::NORTH);
    insert(&mut sim, from, "iron-plate", 400);
    ticks_until(&mut sim, 2000, |s| container_count(s, to, "iron-plate") == 2);
    for _ in 0..3600 {
        sim.step(&[]);
    }
    let moved = container_count(&sim, to, "iron-plate") - 2;
    // Two plates every 70-tick swing.
    assert!((102..=104).contains(&moved), "moved {moved}");
}

#[test]
fn inserter_filters_pick_only_their_items() {
    let d = game!();
    let mut sim = flat_world(d);
    let from = place(&mut sim, "iron-chest", 2, 0, Direction::NORTH);
    let ins = place(&mut sim, "burner-inserter", 2, 1, Direction::NORTH);
    let to = place(&mut sim, "iron-chest", 2, 2, Direction::NORTH);
    insert(&mut sim, ins, "coal", 5);
    let copper = item(&sim, "copper-plate");
    input(&mut sim, InputAction::OpenEntity(Some(MapPosition::tile_center(TilePosition::new(2, 1)))));
    input(&mut sim, InputAction::SetInserterFilter { index: 0, item: Some(copper) });
    input(&mut sim, InputAction::SetInserterFilterMode { use_filters: true, blacklist: false });
    insert(&mut sim, from, "iron-plate", 10);
    insert(&mut sim, from, "copper-plate", 10);
    for _ in 0..1200 {
        sim.step(&[]);
    }
    assert_eq!(container_count(&sim, to, "iron-plate"), 0);
    assert!(container_count(&sim, to, "copper-plate") > 0);
}

#[test]
fn electric_drill_mines_half_an_ore_per_second() {
    let d = game!();
    let mut sim = flat_world(d);
    steam_power(&mut sim, 1);
    for x in 3..8 {
        for y in 2..7 {
            ore(&mut sim, "copper-ore", x, y, 1000);
        }
    }
    place(&mut sim, "small-electric-pole", 2, 1, Direction::NORTH);
    // 3x3 drill at (4..7, 3..6) facing north drops at (0, -1.85): into tile (5, 2).
    place(&mut sim, "electric-mining-drill", 4, 3, Direction::NORTH);
    let chest = place(&mut sim, "iron-chest", 5, 2, Direction::NORTH);
    let t = ticks_until(&mut sim, 2000, |s| container_count(s, chest, "copper-ore") == 1);
    assert!(t <= 125, "first ore after {t}");
    let t = ticks_until(&mut sim, 5000, |s| container_count(s, chest, "copper-ore") == 11);
    // mining_time 1 / mining_speed 0.5 = 2 s.
    assert_eq!(t, 1200);
}

#[test]
fn low_power_slows_machines_down() {
    let d = game!();
    let mut sim = flat_world(d);
    steam_power(&mut sim, 1);
    // One steam engine (900 kW) and twelve electric drills (90 kW each, 1080 kW).
    for x in 0..40 {
        for y in 5..8 {
            ore(&mut sim, "iron-ore", x, y, 10_000);
        }
    }
    let mut chests = Vec::new();
    for i in 0..12 {
        let x = i * 3;
        place(&mut sim, "electric-mining-drill", x, 5, Direction::NORTH);
        chests.push(place(&mut sim, "iron-chest", x + 1, 4, Direction::NORTH));
        if i % 2 == 0 {
            place(&mut sim, "small-electric-pole", x + 2, 3, Direction::NORTH);
        }
    }
    place(&mut sim, "small-electric-pole", 1, 0, Direction::NORTH);
    for _ in 0..600 {
        sim.step(&[]);
    }
    let start: u32 = chests.iter().map(|c| container_count(&sim, *c, "iron-ore")).sum();
    for _ in 0..3600 {
        sim.step(&[]);
    }
    let mined: u32 = chests.iter().map(|c| container_count(&sim, *c, "iron-ore")).sum::<u32>() - start;
    let sat = sim.power.electric_networks[0].satisfaction().to_f64_lossy();
    assert!((0.82..0.85).contains(&sat), "satisfaction {sat}");
    // Full power would be 12 × 30 = 360 per minute; 900/1080 of that is 300.
    assert!((295..=305).contains(&mined), "mined {mined}");
}

#[test]
fn freeplay_start_with_player_inputs_only() {
    let d = game!();
    let mut sim = flat_world(d);
    // Freeplay starting inventory.
    for (name, n) in [("iron-plate", 8), ("wood", 1), ("burner-mining-drill", 1), ("stone-furnace", 1)] {
        give(&mut sim, name, n);
    }
    for x in 2..6 {
        for y in 2..6 {
            ore(&mut sim, "iron-ore", x, y, 500);
        }
    }
    // Within the character's 2.7 tile resource reach.
    ore(&mut sim, "coal", -2, 0, 100);
    let at = |x: i32, y: i32| MapPosition::tile_center(TilePosition::new(x, y));

    // Hand mine 10 coal (2 s each... coal mining_time 1).
    input(&mut sim, InputAction::SetMining(Some(at(-2, 0))));
    ticks_until(&mut sim, 5000, |s| inventory_count(s, "coal") == 10);
    input(&mut sim, InputAction::SetMining(None));

    // Build the drill on the iron and a furnace at its output, then fuel both.
    let drill_item = item(&sim, "burner-mining-drill");
    input(
        &mut sim,
        InputAction::Build {
            item: drill_item,
            position: MapPosition::new(3 * 256, 4 * 256),
            direction: Direction::NORTH,
        },
    );
    assert_eq!(inventory_count(&sim, "burner-mining-drill"), 0, "drill was built");
    let furnace_item = item(&sim, "stone-furnace");
    // Drill (2..4, 3..5) drops at (2.5, 2.7): a furnace covering (2..4, 1..3) catches it.
    input(
        &mut sim,
        InputAction::Build {
            item: furnace_item,
            position: MapPosition::new(3 * 256, 2 * 256),
            direction: Direction::NORTH,
        },
    );
    assert_eq!(inventory_count(&sim, "stone-furnace"), 0, "furnace was built");
    let coal = item(&sim, "coal");
    input(&mut sim, InputAction::TransferToEntity { position: at(2, 3), item: coal, count: 5 });
    input(&mut sim, InputAction::TransferToEntity { position: at(2, 1), item: coal, count: 5 });
    assert_eq!(inventory_count(&sim, "coal"), 0);

    // After a minute the furnace has made plates; take them and hand craft gears.
    for _ in 0..3600 {
        sim.step(&[]);
    }
    input(&mut sim, InputAction::TakeFromEntity { position: at(2, 1) });
    let plates = inventory_count(&sim, "iron-plate");
    assert!(plates >= 8 + 10, "plates: {plates}");
    let gear = sim.prototypes().recipe_id("iron-gear-wheel").unwrap();
    input(&mut sim, InputAction::Craft { recipe: gear, count: 4 });
    ticks_until(&mut sim, 1000, |s| inventory_count(s, "iron-gear-wheel") == 4);

    // Mine the furnace back: it returns itself and its contents.
    input(&mut sim, InputAction::SetMining(Some(at(2, 1))));
    ticks_until(&mut sim, 1000, |s| inventory_count(s, "stone-furnace") == 1);
}

#[test]
fn character_runs_8_9_tiles_per_second() {
    let d = game!();
    let mut sim = flat_world(d);
    let start = sim.player(0).unwrap().character.as_ref().unwrap().position();
    input(&mut sim, InputAction::SetWalking(Some(Direction::EAST)));
    for _ in 0..59 {
        sim.step(&[]);
    }
    let end = sim.player(0).unwrap().character.as_ref().unwrap().position();
    // 0.15 tiles/tick truncated to 38/256: 60 ticks = 2280/256 = 8.906 tiles.
    assert_eq!(end.x - start.x, 38 * 60);
    assert_eq!(end.y, start.y);
}

#[test]
fn diagonal_walking_is_normalised() {
    let d = game!();
    let mut sim = flat_world(d);
    let start = sim.player(0).unwrap().character.as_ref().unwrap().position();
    input(&mut sim, InputAction::SetWalking(Some(Direction(6))));
    for _ in 0..59 {
        sim.step(&[]);
    }
    let end = sim.player(0).unwrap().character.as_ref().unwrap().position();
    // 0.15 × cos 45° = 0.106 tiles/tick per axis, truncated to 27/256.
    assert_eq!(end.x - start.x, 27 * 60);
    assert_eq!(end.y - start.y, 27 * 60);
}

mod cursor {
    use super::*;
    use factorio_sim::input::{EntityInventory, MouseButton, SlotRef};

    fn cursor(sim: &Simulation) -> Option<(String, u32)> {
        let c = sim.player(0).unwrap().character.as_ref().unwrap().cursor?;
        Some((sim.prototypes().item(c.item).name.clone(), c.count))
    }

    fn slot_of(sim: &Simulation, name: &str) -> u16 {
        let i = item(sim, name);
        let inv = &sim.player(0).unwrap().character.as_ref().unwrap().inventory;
        inv.slots().iter().position(|s| s.is_some_and(|s| s.item == i)).unwrap() as u16
    }

    fn click(sim: &mut Simulation, slot: SlotRef, button: MouseButton, shift: bool, ctrl: bool) {
        input(sim, InputAction::ClickSlot { slot, button, shift, ctrl });
    }

    #[test]
    fn left_click_picks_up_and_puts_down_a_stack() {
        let d = game!();
        let mut sim = flat_world(d);
        give(&mut sim, "iron-plate", 150);
        let s = slot_of(&sim, "iron-plate");
        click(&mut sim, SlotRef::Character(s), MouseButton::Left, false, false);
        assert_eq!(cursor(&sim), Some(("iron-plate".into(), 100)));
        assert_eq!(inventory_count(&sim, "iron-plate"), 50);
        input(&mut sim, InputAction::ClearCursor);
        assert_eq!(cursor(&sim), None);
        assert_eq!(inventory_count(&sim, "iron-plate"), 150);
    }

    fn hand(sim: &Simulation) -> Option<usize> {
        sim.player(0).unwrap().character.as_ref().unwrap().inventory.reserved()
    }

    #[test]
    fn picked_up_stack_leaves_the_hand_and_q_returns_it_there() {
        let d = game!();
        let mut sim = flat_world(d);
        give(&mut sim, "iron-plate", 100);
        give(&mut sim, "copper-plate", 10);
        let s = slot_of(&sim, "iron-plate");
        click(&mut sim, SlotRef::Character(s), MouseButton::Left, false, false);
        assert_eq!(hand(&sim), Some(s as usize));
        // New items and the inventory's sorting leave the hand's slot empty.
        give(&mut sim, "coal", 5);
        let inv = &sim.player(0).unwrap().character.as_ref().unwrap().inventory;
        assert!(inv.slot(s as usize).is_none());
        assert_eq!(hand(&sim), Some(s as usize));
        // Q puts the stack back.
        input(&mut sim, InputAction::ClearCursor);
        assert_eq!(inventory_count(&sim, "iron-plate"), 100);
        assert_eq!(hand(&sim), None);
    }

    #[test]
    fn placing_the_stack_elsewhere_drops_the_hand() {
        let d = game!();
        let mut sim = flat_world(d);
        give(&mut sim, "iron-plate", 100);
        let s = slot_of(&sim, "iron-plate");
        click(&mut sim, SlotRef::Character(s), MouseButton::Left, false, false);
        click(&mut sim, SlotRef::Character(s + 5), MouseButton::Left, false, false);
        assert_eq!(hand(&sim), None);
        assert_eq!(cursor(&sim), None);
        let s = slot_of(&sim, "iron-plate");
        // Taking half a stack leaves items in the slot: no hand.
        click(&mut sim, SlotRef::Character(s), MouseButton::Right, false, false);
        assert_eq!(hand(&sim), None);
    }

    fn chest_slot(sim: &Simulation, id: EntityId, i: usize) -> u32 {
        match &sim.entity(id).unwrap().state {
            EntityState::Container(inv) => inv.slot(i).map_or(0, |s| s.count),
            _ => 0,
        }
    }

    #[test]
    fn filtered_slots_only_take_their_item() {
        let d = game!();
        let mut sim = flat_world(d);
        let plate = item(&sim, "iron-plate");
        input(&mut sim, InputAction::SetSlotFilter { slot: 0, item: Some(plate) });
        give(&mut sim, "coal", 10);
        let inv = &sim.player(0).unwrap().character.as_ref().unwrap().inventory;
        assert!(inv.slot(0).is_none(), "coal must not go into the iron plate slot");
        give(&mut sim, "iron-plate", 10);
        let inv = &sim.player(0).unwrap().character.as_ref().unwrap().inventory;
        assert_eq!(inv.slot(0).map(|s| (s.item, s.count)), Some((plate, 10)));
        input(&mut sim, InputAction::SetSlotFilter { slot: 0, item: None });
        assert_eq!(sim.player(0).unwrap().character.as_ref().unwrap().inventory.filter(0), None);
    }

    #[test]
    fn container_limit_stops_insertion_past_the_bar() {
        let d = game!();
        let mut sim = flat_world(d);
        let chest = place(&mut sim, "iron-chest", 2, 0, Direction::NORTH);
        input(&mut sim, InputAction::OpenEntity(Some(MapPosition::tile_center(TilePosition::new(2, 0)))));
        input(&mut sim, InputAction::SetContainerLimit(Some(2)));
        let plate = item(&sim, "iron-plate");
        let n = sim.insert_into_entity(chest, plate, 500, factorio_sim::world::InsertSource::Automated);
        assert_eq!(n, 200);
        assert!(!sim.entity_wants(chest, plate));
        input(&mut sim, InputAction::SetContainerLimit(None));
        assert!(sim.entity_wants(chest, plate));
    }

    // The character's inventory sorts itself (as with the game's default setting), so
    // spreading shows in a chest.
    #[test]
    fn left_drag_spreads_the_stack_evenly() {
        let d = game!();
        let mut sim = flat_world(d);
        give(&mut sim, "iron-plate", 100);
        let chest = place(&mut sim, "iron-chest", 2, 0, Direction::NORTH);
        input(&mut sim, InputAction::OpenEntity(Some(MapPosition::tile_center(TilePosition::new(2, 0)))));
        let s = slot_of(&sim, "iron-plate");
        click(&mut sim, SlotRef::Character(s), MouseButton::Left, false, false);
        let slot = |i| SlotRef::Opened(EntityInventory::Main, i);
        input(&mut sim, InputAction::SpreadCursor { slots: vec![slot(0)] });
        assert_eq!(chest_slot(&sim, chest, 0), 100);
        assert_eq!(cursor(&sim), None);
        input(&mut sim, InputAction::SpreadCursor { slots: vec![slot(0), slot(5)] });
        assert_eq!((chest_slot(&sim, chest, 0), chest_slot(&sim, chest, 5)), (50, 50));
        input(&mut sim, InputAction::SpreadCursor { slots: vec![slot(0), slot(5), slot(9)] });
        let counts = (chest_slot(&sim, chest, 0), chest_slot(&sim, chest, 5), chest_slot(&sim, chest, 9));
        assert_eq!(counts, (34, 33, 33));
        input(&mut sim, InputAction::EndSpread);
        assert_eq!(container_count(&sim, chest, "iron-plate"), 100);
    }

    #[test]
    fn right_click_takes_half_and_places_one() {
        let d = game!();
        let mut sim = flat_world(d);
        give(&mut sim, "iron-plate", 7);
        let s = slot_of(&sim, "iron-plate");
        click(&mut sim, SlotRef::Character(s), MouseButton::Right, false, false);
        assert_eq!(cursor(&sim), Some(("iron-plate".into(), 4)));
        let chest = place(&mut sim, "wooden-chest", 2, 0, Direction::NORTH);
        input(&mut sim, InputAction::OpenEntity(Some(MapPosition::tile_center(TilePosition::new(2, 0)))));
        click(&mut sim, SlotRef::Opened(EntityInventory::Main, 0), MouseButton::Right, false, false);
        assert_eq!(cursor(&sim), Some(("iron-plate".into(), 3)));
        assert_eq!(container_count(&sim, chest, "iron-plate"), 1);
        click(&mut sim, SlotRef::Opened(EntityInventory::Main, 0), MouseButton::Left, false, false);
        assert_eq!(cursor(&sim), None);
        assert_eq!(container_count(&sim, chest, "iron-plate"), 4);
    }

    #[test]
    fn shift_and_ctrl_click_transfer_to_the_open_entity() {
        let d = game!();
        let mut sim = flat_world(d);
        give(&mut sim, "iron-plate", 250);
        let chest = place(&mut sim, "iron-chest", 2, 0, Direction::NORTH);
        input(&mut sim, InputAction::OpenEntity(Some(MapPosition::tile_center(TilePosition::new(2, 0)))));
        let s = slot_of(&sim, "iron-plate");
        click(&mut sim, SlotRef::Character(s), MouseButton::Left, true, false);
        assert_eq!(container_count(&sim, chest, "iron-plate"), 100);
        let s = slot_of(&sim, "iron-plate");
        click(&mut sim, SlotRef::Character(s), MouseButton::Left, false, true);
        assert_eq!(container_count(&sim, chest, "iron-plate"), 250);
        assert_eq!(inventory_count(&sim, "iron-plate"), 0);
        // And back: shift-click a chest slot moves that stack to the character.
        click(&mut sim, SlotRef::Opened(EntityInventory::Main, 0), MouseButton::Left, true, false);
    }

    #[test]
    fn furnace_slots_only_accept_valid_items() {
        let d = game!();
        let mut sim = flat_world(d);
        give(&mut sim, "iron-plate", 10);
        give(&mut sim, "coal", 10);
        let furnace = place(&mut sim, "stone-furnace", 2, 0, Direction::NORTH);
        input(&mut sim, InputAction::OpenEntity(Some(MapPosition::tile_center(TilePosition::new(2, 0)))));
        // Coal into the fuel slot works.
        let picked = item(&sim, "coal");
        input(&mut sim, InputAction::PickItem(picked));
        click(&mut sim, SlotRef::Opened(EntityInventory::Fuel, 0), MouseButton::Left, false, false);
        assert_eq!(cursor(&sim), None);
        // Iron plates are not fuel; they can go in as steel ingredients though.
        let picked = item(&sim, "iron-plate");
        input(&mut sim, InputAction::PickItem(picked));
        click(&mut sim, SlotRef::Opened(EntityInventory::Fuel, 0), MouseButton::Left, false, false);
        assert_eq!(cursor(&sim), Some(("iron-plate".into(), 10)));
        // Steel is locked until steel processing is researched, so the plates stay.
        click(&mut sim, SlotRef::Opened(EntityInventory::Input, 0), MouseButton::Left, false, false);
        assert_eq!(cursor(&sim), Some(("iron-plate".into(), 10)));
        let steel = sim.prototypes().technology_id("steel-processing").unwrap();
        sim.finish_research(steel);
        click(&mut sim, SlotRef::Opened(EntityInventory::Input, 0), MouseButton::Left, false, false);
        assert_eq!(cursor(&sim), None);
        // The fuelled furnace starts on steel (5 plates) straight away.
        let EntityState::Crafter(c) = &sim.entity(furnace).unwrap().state else { panic!() };
        assert!(c.crafting);
        assert_eq!(c.input.count(item(&sim, "iron-plate")), 5);
        // Nothing can be put into the output.
        let picked = item(&sim, "coal");
        input(&mut sim, InputAction::PickItem(picked));
        click(&mut sim, SlotRef::Opened(EntityInventory::Output, 0), MouseButton::Left, false, false);
        assert!(cursor(&sim).is_none() || cursor(&sim).unwrap().0 == "coal");
    }

    #[test]
    fn building_uses_the_cursor_and_refills_it() {
        let d = game!();
        let mut sim = flat_world(d);
        give(&mut sim, "wooden-chest", 51);
        let picked = item(&sim, "wooden-chest");
        input(&mut sim, InputAction::PickItem(picked));
        assert_eq!(cursor(&sim), Some(("wooden-chest".into(), 50)));
        let chest = item(&sim, "wooden-chest");
        input(
            &mut sim,
            InputAction::Build {
                item: chest,
                position: MapPosition::tile_center(TilePosition::new(3, 3)),
                direction: Direction::NORTH,
            },
        );
        assert_eq!(cursor(&sim), Some(("wooden-chest".into(), 49)));
        assert_eq!(inventory_count(&sim, "wooden-chest"), 1);
    }

    #[test]
    fn ctrl_click_on_a_building_inserts_or_takes() {
        let d = game!();
        let mut sim = flat_world(d);
        give(&mut sim, "coal", 20);
        let furnace = place(&mut sim, "stone-furnace", 2, 0, Direction::NORTH);
        let at = MapPosition::tile_center(TilePosition::new(2, 0));
        let picked = item(&sim, "coal");
        input(&mut sim, InputAction::PickItem(picked));
        input(&mut sim, InputAction::FastTransfer { position: at, half: false });
        assert_eq!(cursor(&sim), None);
        let EntityState::Crafter(c) = &sim.entity(furnace).unwrap().state else { panic!() };
        assert_eq!(c.energy.burner().unwrap().fuel.count(item(&sim, "coal")), 20);
    }

    #[test]
    fn shift_click_crafts_as_many_as_possible() {
        let d = game!();
        let mut sim = flat_world(d);
        give(&mut sim, "iron-plate", 11);
        let gear = sim.prototypes().recipe_id("iron-gear-wheel").unwrap();
        input(&mut sim, InputAction::Craft { recipe: gear, count: u32::MAX });
        let queued: u32 = sim.player(0).unwrap().character.as_ref().unwrap().queue.iter().map(|j| j.count).sum();
        assert_eq!(queued, 5);
        assert_eq!(inventory_count(&sim, "iron-plate"), 1);
    }
}

#[test]
fn cheat_mode_crafts_instantly_and_free() {
    let d = game!();
    let mut sim = flat_world(d);
    input(&mut sim, InputAction::SetCheatMode(true));
    let r = sim.prototypes().recipe_id("assembling-machine-1").unwrap();
    input(&mut sim, InputAction::Craft { recipe: r, count: 3 });
    assert_eq!(inventory_count(&sim, "assembling-machine-1"), 3);
    // F1: a stack of everything, overflowing into chests.
    input(&mut sim, InputAction::CheatAllItems);
    let total_items = sim.prototypes().items.len();
    let in_chests: usize = sim
        .entities()
        .filter_map(|(_, e)| match &e.state {
            EntityState::Container(inv) => Some(inv.slots().iter().filter(|s| s.is_some()).count()),
            _ => None,
        })
        .sum();
    let carried =
        sim.player(0).unwrap().character.as_ref().unwrap().inventory.slots().iter().filter(|s| s.is_some()).count();
    // One extra slot: the 3 crafted assemblers plus a full stack of them.
    assert_eq!(carried + in_chests, total_items + 1);
}

mod research {
    use super::*;
    use factorio_sim::proto::TechId;

    fn tech(sim: &Simulation, name: &str) -> TechId {
        sim.prototypes().technology_id(name).unwrap_or_else(|| panic!("no technology {name}"))
    }

    fn enabled(sim: &Simulation, recipe: &str) -> bool {
        sim.research().recipe_enabled(sim.prototypes().recipe_id(recipe).unwrap())
    }

    /// Finishes every technology that is researched by a trigger, and their prerequisites.
    fn research_triggers(sim: &mut Simulation) {
        loop {
            let db = sim.prototypes_arc();
            let next = db
                .technology_ids()
                .find(|t| db.technology(*t).trigger.is_some() && sim.research().is_available(&db, *t));
            match next {
                Some(t) => sim.finish_research(t),
                None => break,
            }
        }
    }

    fn researched(sim: &Simulation, name: &str) -> bool {
        sim.research().is_researched(tech(sim, name))
    }

    /// A lab next to a pole on steam power, filled with automation science packs.
    fn powered_labs(sim: &mut Simulation, labs: i32, packs: u32) -> Vec<EntityId> {
        steam_power(sim, 1);
        place(sim, "small-electric-pole", 0, 1, Direction::NORTH);
        let ids: Vec<EntityId> = (0..labs).map(|i| place(sim, "lab", 1 + 3 * i, 1, Direction::NORTH)).collect();
        for (i, lab) in ids.iter().enumerate() {
            if i > 0 {
                place(sim, "small-electric-pole", 3 * i as i32, 4, Direction::NORTH);
            }
            insert(sim, *lab, "automation-science-pack", packs);
        }
        ids
    }

    #[test]
    fn freeplay_starts_with_the_games_unlocked_recipes() {
        let d = game!();
        let sim = flat_world(d);
        let db = sim.prototypes();
        let mut unlocked: Vec<&str> = db
            .recipe_ids()
            .filter(|r| sim.research().recipe_enabled(*r) && !db.recipe(*r).hidden)
            .map(|r| db.recipe(r).name.as_str())
            .collect();
        unlocked.sort();
        assert_eq!(
            unlocked,
            [
                "burner-inserter",
                "burner-mining-drill",
                "copper-plate",
                "firearm-magazine",
                "iron-chest",
                "iron-gear-wheel",
                "iron-plate",
                "light-armor",
                "stone-brick",
                "stone-furnace",
                "transport-belt",
                "wooden-chest",
            ]
        );
    }

    #[test]
    fn locked_recipes_cannot_be_crafted_or_used() {
        let d = game!();
        let mut sim = flat_world(d);
        give(&mut sim, "copper-plate", 10);
        let cable = sim.prototypes().recipe_id("copper-cable").unwrap();
        input(&mut sim, InputAction::Craft { recipe: cable, count: 1 });
        assert!(sim.player(0).unwrap().character.as_ref().unwrap().queue.is_empty());
        // Unlocking it (electronics) makes it craftable.
        let electronics = tech(&sim, "electronics");
        sim.finish_research(electronics);
        assert!(enabled(&sim, "copper-cable"));
        input(&mut sim, InputAction::Craft { recipe: cable, count: 1 });
        assert_eq!(sim.player(0).unwrap().character.as_ref().unwrap().queue.len(), 1);
    }

    #[test]
    fn steam_power_is_researched_by_smelting_50_iron_plates() {
        let d = game!();
        let mut sim = flat_world(d);
        assert!(!enabled(&sim, "offshore-pump"));
        let furnace = place(&mut sim, "stone-furnace", 2, 0, Direction::NORTH);
        insert(&mut sim, furnace, "coal", 20);
        insert(&mut sim, furnace, "iron-ore", 50);
        // Stone furnace: 3.2 s at crafting speed 1 = 192 ticks per plate.
        let t = ticks_until(&mut sim, 20_000, |s| researched(s, "steam-power"));
        let plates = container_count(&sim, furnace, "iron-plate");
        assert_eq!(plates, 50);
        assert_eq!(t, 50 * 192);
        for r in ["offshore-pump", "boiler", "steam-engine", "pipe"] {
            assert!(enabled(&sim, r), "{r} not unlocked");
        }
    }

    #[test]
    fn trigger_progress_only_counts_once_available() {
        let d = game!();
        let mut sim = flat_world(d);
        // automation-science-pack needs a crafted lab, after electronics and steam power.
        let lab = sim.prototypes().item_id("lab").unwrap();
        sim.research_trigger(factorio_sim::research::TriggerEvent::Crafted(lab, 1));
        assert!(!researched(&sim, "automation-science-pack"));
        for t in ["electronics", "steam-power"] {
            let t = tech(&sim, t);
            sim.finish_research(t);
        }
        sim.research_trigger(factorio_sim::research::TriggerEvent::Crafted(lab, 1));
        assert!(researched(&sim, "automation-science-pack"));
    }

    #[test]
    fn one_lab_researches_automation_in_100_seconds() {
        let d = game!();
        let mut sim = flat_world(d);
        research_triggers(&mut sim);
        let labs = powered_labs(&mut sim, 1, 10);
        let automation = tech(&sim, "automation");
        input(&mut sim, InputAction::QueueResearch { tech: automation, front: false });
        assert_eq!(sim.research().current(), Some(automation));
        // 10 units × 10 s at lab speed 1. The lab's buffer fills on the first tick.
        let t = 1 + ticks_until(&mut sim, 10_000, |s| researched(s, "automation"));
        assert!((6000..=6002).contains(&t), "took {t} ticks");
        assert!(enabled(&sim, "assembling-machine-1"));
        assert!(enabled(&sim, "long-handed-inserter"));
        let EntityState::Lab(l) = &sim.entity(labs[0]).unwrap().state else { panic!() };
        assert!(l.input.is_empty(), "all 10 packs used: {:?}", l.input);
        assert_eq!(l.opened[0], 0);
        assert_eq!(sim.research().current(), None);
    }

    #[test]
    fn two_labs_research_twice_as_fast() {
        let d = game!();
        let mut sim = flat_world(d);
        research_triggers(&mut sim);
        powered_labs(&mut sim, 2, 10);
        let automation = tech(&sim, "automation");
        input(&mut sim, InputAction::QueueResearch { tech: automation, front: false });
        let t = 1 + ticks_until(&mut sim, 10_000, |s| researched(s, "automation"));
        assert!((3000..=3002).contains(&t), "took {t} ticks");
    }

    #[test]
    fn queueing_adds_missing_prerequisites_first() {
        let d = game!();
        let mut sim = flat_world(d);
        let auto2 = tech(&sim, "automation-2");
        // Trigger technologies cannot be queued.
        input(&mut sim, InputAction::QueueResearch { tech: auto2, front: false });
        assert!(sim.research().queue.is_empty());
        research_triggers(&mut sim);
        input(&mut sim, InputAction::QueueResearch { tech: auto2, front: false });
        let q = &sim.research().queue;
        assert_eq!(q.last(), Some(&auto2));
        for (i, t) in q.iter().enumerate() {
            for p in &sim.prototypes().technology(*t).prerequisites {
                let pos = q.iter().position(|x| x == p);
                assert!(sim.research().is_researched(*p) || pos.is_some_and(|pi| pi < i));
            }
        }
        // Removing a prerequisite removes what depends on it.
        let first = q[0];
        input(&mut sim, InputAction::DequeueResearch(first));
        assert!(!sim.research().queue.contains(&auto2));
    }

    #[test]
    fn infinite_technologies_use_the_count_formula() {
        let d = game!();
        let mut sim = flat_world(d);
        let t = tech(&sim, "mining-productivity-4");
        // 2500 * (L - 3) units of 60 s.
        let per_unit = 60 * 60 * Fixed::ONE.raw() as u128;
        assert_eq!(sim.research().work_needed(sim.prototypes(), t), 2500 * per_unit);
        sim.finish_research(t);
        assert!(!sim.research().is_researched(t));
        assert_eq!(sim.research().level[t.index()], 5);
        assert_eq!(sim.research().work_needed(sim.prototypes(), t), 5000 * per_unit);
    }

    #[test]
    fn steel_axe_doubles_hand_mining_speed() {
        let d = game!();
        let mut sim = flat_world(d);
        let axe = tech(&sim, "steel-axe");
        sim.finish_research(axe);
        ore(&mut sim, "iron-ore", 1, 0, 50);
        input(&mut sim, InputAction::SetMining(Some(MapPosition::tile_center(TilePosition::new(1, 0)))));
        let t = 1 + ticks_until(&mut sim, 1000, |s| inventory_count(s, "iron-ore") == 1);
        // Mining speed 0.5 × (1 + 1) = 1: one second.
        assert_eq!(t, 60);
    }

    #[test]
    fn mining_productivity_gives_an_extra_ore_every_tenth() {
        let d = game!();
        let mut sim = flat_world(d);
        let prod = tech(&sim, "mining-productivity-1");
        sim.finish_research(prod);
        let tiles = [(10, 10), (11, 10), (10, 11), (11, 11)];
        for (x, y) in tiles {
            ore(&mut sim, "iron-ore", x, y, 1000);
        }
        let drill = place(&mut sim, "burner-mining-drill", 10, 10, Direction::NORTH);
        let chest = place(&mut sim, "iron-chest", 10, 9, Direction::NORTH);
        insert(&mut sim, drill, "coal", 20);
        // +10%: the bar fills on every tenth ore, which then yields two.
        let _ = ticks_until(&mut sim, 20_000, |s| container_count(s, chest, "iron-ore") >= 11);
        let mined: u32 =
            tiles.iter().map(|(x, y)| 1000 - sim.surface.resource(TilePosition::new(*x, *y)).unwrap().amount).sum();
        assert_eq!(mined, 10);
    }

    #[test]
    fn character_inventory_bonus_adds_slots() {
        let d = game!();
        let mut sim = flat_world(d);
        let before = sim.player(0).unwrap().character.as_ref().unwrap().inventory.len();
        let toolbelt = tech(&sim, "toolbelt");
        sim.finish_research(toolbelt);
        let after = sim.player(0).unwrap().character.as_ref().unwrap().inventory.len();
        assert_eq!(after, before + 10);
    }
}
