//! Map generation from the game's noise expressions (skipped without an install).

use factorio_sim::map::{ChunkPosition, TilePosition};
use factorio_sim::surface::Surface;

#[test]
fn nauvis_expressions_compile_and_generate_deterministically() {
    let Ok(config) = factorio_data::Config::load() else { return };
    let Ok(data) = factorio_data::load_game_data(&config) else {
        eprintln!("skipping: no Factorio install");
        return;
    };
    let db = factorio_data::typed::build_prototype_db(&data).unwrap();
    let settings = factorio_data::mapgen::planet_mapgen(&data, &db, 1234);
    let noise = settings.noise.as_ref().unwrap();
    assert!(noise.tiles.len() >= 20, "Nauvis lists {} tiles", noise.tiles.len());
    assert!(noise.resources.len() >= 5);

    let mut a = Surface::new(settings.clone());
    assert_eq!(a.noise_error, None);
    let mut b = Surface::new(settings);
    // Generation order must not matter.
    for (x, y) in [(0, 0), (3, -2), (-5, 4)] {
        a.ensure_chunk(ChunkPosition { x, y });
    }
    for (x, y) in [(-5, 4), (3, -2), (0, 0)] {
        b.ensure_chunk(ChunkPosition { x, y });
    }
    for (x, y) in [(0, 0), (3, -2), (-5, 4)] {
        let c = ChunkPosition { x, y };
        assert_eq!(a.chunks().find(|(p, _)| *p == c).unwrap().1, b.chunks().find(|(p, _)| *p == c).unwrap().1);
    }
    // The spawn is land.
    let water = db.tile_id("water").unwrap();
    let deep = db.tile_id("deepwater").unwrap();
    let t = a.tile(TilePosition::new(0, 0)).unwrap();
    assert!(t != water && t != deep);
}

#[test]
fn trees_and_rocks_are_generated_as_minable_entities() {
    use factorio_sim::input::{InputAction, PlayerInput};
    use factorio_sim::proto::ItemOrFluid;
    use factorio_sim::world::{EntityState, Simulation};
    let Ok(config) = factorio_data::Config::load() else { return };
    let Ok(data) = factorio_data::load_game_data(&config) else { return };
    let db = std::sync::Arc::new(factorio_data::typed::build_prototype_db(&data).unwrap());
    let mut sim = Simulation::new(db.clone(), factorio_data::mapgen::planet_mapgen(&data, &db, 1));
    sim.step(&[PlayerInput::new(0, InputAction::JoinGame)]);
    let mut kinds = std::collections::BTreeMap::new();
    for (_, e) in sim.entities() {
        if let EntityState::Static { .. } = e.state {
            *kinds.entry(db.entity(e.proto).kind.clone()).or_insert(0) += 1;
        }
    }
    assert!(kinds.get("tree").copied().unwrap_or(0) > 100, "{kinds:?}");
    assert!(kinds.get("simple-entity").copied().unwrap_or(0) > 0, "{kinds:?}");
    // Trees give wood when mined.
    let tree = sim.entities().find(|(_, e)| db.entity(e.proto).kind == "tree").unwrap().1;
    let wood = db.item_id("wood").unwrap();
    let results = &db.entity(tree.proto).minable.as_ref().unwrap().results;
    assert!(results.iter().any(|r| r.what == ItemOrFluid::Item(wood)));
}
