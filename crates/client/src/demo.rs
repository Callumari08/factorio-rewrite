//! `FACTORIO_REWRITE_DEMO=1`: builds a small burner-phase factory on the nearest iron
//! patch using ordinary (cheat) inputs, and adds the sandbox kit. Handy for checking that
//! everything works without playing through the start.

use factorio_sim::input::InputAction;
use factorio_sim::map::{Direction, MapPosition, TilePosition};
use factorio_sim::{PlayerInput, Simulation};

pub fn build(sim: &mut Simulation) {
    let db = sim.prototypes_arc();
    let Some(iron) = db.entity_id("iron-ore") else { return };
    let is_iron = |sim: &Simulation, x: i32, y: i32| {
        sim.surface.resource(TilePosition::new(x, y)).is_some_and(|r| r.proto == iron)
    };
    // Nearest spot with a 4x2 block of iron for two drills.
    let mut best: Option<(i64, TilePosition)> = None;
    for y in -60..60 {
        for x in -60..60 {
            let ok = (0..4).all(|dx| (0..2).all(|dy| is_iron(sim, x + dx, y + dy)));
            let d = (x as i64).pow(2) + (y as i64).pow(2);
            if ok && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, TilePosition::new(x, y)));
            }
        }
    }
    let Some((_, o)) = best else { return };

    let mut inputs = Vec::new();
    let mut place = |name: &str, x: i32, y: i32, dir: Direction| {
        if let Some(e) = db.entity_id(name) {
            let pos = db.entity(e).position_for_tile(TilePosition::new(x, y), dir);
            inputs
                .push(PlayerInput::new(0, InputAction::CheatPlaceEntity { entity: e, position: pos, direction: dir }));
        }
    };
    place("burner-mining-drill", o.x, o.y, Direction::NORTH);
    place("burner-mining-drill", o.x + 2, o.y, Direction::NORTH);
    for x in o.x - 6..=o.x + 3 {
        place("transport-belt", x, o.y - 1, Direction::WEST);
    }
    // The line ends in a right turn (west → north) and then a left turn (north → west).
    place("transport-belt", o.x - 7, o.y - 1, Direction::NORTH);
    place("transport-belt", o.x - 7, o.y - 2, Direction::NORTH);
    place("transport-belt", o.x - 7, o.y - 3, Direction::WEST);
    place("transport-belt", o.x - 8, o.y - 3, Direction::WEST);
    place("burner-inserter", o.x - 4, o.y - 2, Direction::SOUTH);
    place("stone-furnace", o.x - 5, o.y - 4, Direction::NORTH);
    place("burner-inserter", o.x - 4, o.y - 5, Direction::SOUTH);
    place("iron-chest", o.x - 4, o.y - 6, Direction::NORTH);
    sim.step(&inputs);

    let coal = db.item_id("coal").unwrap();
    let mut fuel = Vec::new();
    for (x, y) in [(o.x, o.y), (o.x + 2, o.y), (o.x - 4, o.y - 2), (o.x - 5, o.y - 4), (o.x - 4, o.y - 5)] {
        let position = MapPosition::tile_center(TilePosition::new(x, y));
        fuel.push(PlayerInput::new(0, InputAction::CheatInsert { position, item: coal, count: 10 }));
    }
    sim.step(&fuel);
    build_power(sim);
}

/// Steam power on the nearest shore where the layout fits: offshore pump facing north
/// into the water, boiler and steam engine behind it, a pole and an assembler on gears.
fn build_power(sim: &mut Simulation) {
    let db = sim.prototypes_arc();
    let id = |n: &str| db.entity_id(n);
    let (Some(pump), Some(boiler), Some(engine), Some(pole), Some(asm)) =
        (id("offshore-pump"), id("boiler"), id("steam-engine"), id("small-electric-pole"), id("assembling-machine-1"))
    else {
        return;
    };
    let lab = id("lab");
    let pos = |e, x, y, d| db.entity(e).position_for_tile(TilePosition::new(x, y), d);
    let mut spot = None;
    'search: for r in 0..60i32 {
        for y in -r..=r {
            for x in -r..=r {
                if x.abs() != r && y.abs() != r {
                    continue;
                }
                let fits = sim.can_place(pump, pos(pump, x, y, Direction::NORTH), Direction::NORTH).is_ok()
                    && sim.can_place(boiler, pos(boiler, x, y + 1, Direction::EAST), Direction::EAST).is_ok()
                    && sim.can_place(engine, pos(engine, x + 2, y + 1, Direction::EAST), Direction::EAST).is_ok()
                    && sim.can_place(asm, pos(asm, x + 2, y + 5, Direction::NORTH), Direction::NORTH).is_ok()
                    && lab.is_none_or(|l| {
                        sim.can_place(l, pos(l, x + 5, y + 5, Direction::NORTH), Direction::NORTH).is_ok()
                    });
                if fits {
                    spot = Some((x, y));
                    break 'search;
                }
            }
        }
    }
    let Some((x, y)) = spot else { return };
    let place = |e, tx, ty, d| {
        PlayerInput::new(0, InputAction::CheatPlaceEntity { entity: e, position: pos(e, tx, ty, d), direction: d })
    };
    sim.step(&[
        place(pump, x, y, Direction::NORTH),
        place(boiler, x, y + 1, Direction::EAST),
        place(engine, x + 2, y + 1, Direction::EAST),
        place(pole, x + 3, y + 4, Direction::NORTH),
        place(asm, x + 2, y + 5, Direction::NORTH),
    ]);
    if let (Some(lab), Some(pack)) = (lab, db.item_id("automation-science-pack")) {
        sim.step(&[place(lab, x + 5, y + 5, Direction::NORTH)]);
        let at = MapPosition::tile_center(TilePosition::new(x + 6, y + 6));
        sim.step(&[PlayerInput::new(0, InputAction::CheatInsert { position: at, item: pack, count: 10 })]);
        // Research what the burner phase would have unlocked, and start Automation.
        for name in ["steam-power", "electronics", "automation-science-pack"] {
            if let Some(t) = db.technology_id(name) {
                sim.finish_research(t);
            }
        }
        if let Some(t) = db.technology_id("automation") {
            sim.step(&[PlayerInput::new(0, InputAction::QueueResearch { tech: t, front: false })]);
        }
    }
    let (Some(coal), Some(plate), Some(gear)) =
        (db.item_id("coal"), db.item_id("iron-plate"), db.recipe_id("iron-gear-wheel"))
    else {
        return;
    };
    let at = |tx, ty| MapPosition::tile_center(TilePosition::new(tx, ty));
    sim.step(&[
        PlayerInput::new(0, InputAction::CheatInsert { position: at(x, y + 1), item: coal, count: 50 }),
        PlayerInput::new(0, InputAction::CheatSetRecipe { position: at(x + 3, y + 6), recipe: gear }),
    ]);
    sim.step(&[PlayerInput::new(0, InputAction::CheatInsert { position: at(x + 3, y + 6), item: plate, count: 100 })]);
}
