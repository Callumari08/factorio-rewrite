//! Loads the game data and prints a summary, or one prototype.
//!
//!     cargo run -p factorio-data --example dump
//!     cargo run -p factorio-data --example dump -- item iron-plate

use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("RUST_LOG").is_none() {
        unsafe { std::env::set_var("RUST_LOG", "info") };
    }
    env_logger::init();
    let start = Instant::now();
    let config = factorio_data::Config::load()?;
    let data = factorio_data::load_game_data(&config)?;
    let db = factorio_data::typed::build_prototype_db(&data)?;
    println!("data stage finished in {:.2?}", start.elapsed());

    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [kind, name] = args.as_slice() {
        println!("{:#?}", data.prototype(kind, name));
        return Ok(());
    }

    let types = data.raw.as_table().map(|t| t.len()).unwrap_or(0);
    let total: usize =
        data.raw.as_table().into_iter().flatten().map(|(_, v)| v.as_table().map_or(0, |t| t.len())).sum();
    println!("{types} prototype types, {total} prototypes");
    println!(
        "{} items, {} fluids, {} recipes, {} entities, {} tiles, {} collision layers",
        db.items.len(),
        db.fluids.len(),
        db.recipes.len(),
        db.entities.len(),
        db.tiles.len(),
        db.collision_layers.len()
    );
    for name in [
        "burner-mining-drill",
        "stone-furnace",
        "inserter",
        "transport-belt",
        "steam-engine",
        "boiler",
        "offshore-pump",
    ] {
        if let Some(id) = db.entity_id(name) {
            println!("{:#?}", db.entity(id));
        }
    }
    let mapgen = factorio_data::mapgen::default_mapgen(&db, 0);
    println!(
        "mapgen: {} land tiles, resources: {:?}",
        mapgen.land.len(),
        mapgen.resources.iter().map(|r| &db.entity(r.resource).name).collect::<Vec<_>>()
    );
    Ok(())
}
