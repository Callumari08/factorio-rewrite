//! Locale names from the real game files (skipped without an install).

#[test]
fn english_names_resolve_like_the_game() {
    let Ok(config) = factorio_data::Config::load() else { return };
    let Ok(data) = factorio_data::load_game_data(&config) else {
        eprintln!("skipping: no Factorio install");
        return;
    };
    let l = factorio_data::locale::Locale::load(&data, "en");
    assert_eq!(l.item_name(&data, "iron-plate"), "Iron plate");
    assert_eq!(l.item_name(&data, "stone-furnace"), "Stone furnace");
    assert_eq!(l.item_name(&data, "transport-belt"), "Transport belt");
    assert_eq!(l.recipe_name(&data, "iron-gear-wheel"), "Iron gear wheel");
    assert_eq!(l.entity_name(&data, "burner-mining-drill"), "Burner mining drill");
    assert_eq!(l.fluid_name(&data, "water"), "Water");
    assert_eq!(l.item_group_name("logistics"), "Logistics");
}
