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
}
