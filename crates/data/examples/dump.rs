//! Loads the game data and prints a summary, or one prototype as JSON-ish debug output.
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
    println!("{} items, {} recipes, {} entities", db.items.len(), db.recipes.len(), db.entities.len());
    for name in ["iron-plate", "transport-belt", "electronic-circuit"] {
        if let Some(r) = db.recipes.get(name) {
            println!("recipe {name}: {r:?}");
        }
    }
    if let Some(belt) = db.entities.get("transport-belt") {
        println!("entity transport-belt: {belt:?}");
    }
    if let Some(item) = db.items.get("coal") {
        println!("item coal: {item:?}");
    }
    for e in ["transport-belt", "stone-furnace"] {
        println!("{e} sprite: {:?}", factorio_data::sprite::entity_sprite(&data, e));
    }
    let icon = factorio_data::sprite::item_icon(&data, "iron-plate");
    println!("iron-plate icon: {icon:?}");
    Ok(())
}
