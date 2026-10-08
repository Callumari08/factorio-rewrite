//! Sound definitions from the real game files (skipped without an install).

use factorio_data::sound::*;

#[test]
fn sounds_parse_from_the_game_data() {
    let Ok(config) = factorio_data::Config::load() else { return };
    let Ok(data) = factorio_data::load_game_data(&config) else {
        eprintln!("skipping: no Factorio install");
        return;
    };
    // A working sound with a fade and an audible distance.
    let lab = working_sound(&data, "lab").expect("lab working sound");
    assert_eq!(lab.sound.audible_distance_modifier, 0.7);
    assert_eq!((lab.fade_in_ticks, lab.fade_out_ticks), (4, 20));
    assert!(lab.sound.variations[0].path.ends_with("sound/lab.ogg"));
    // Belts share one sound; inserters play per swing; drills have variations.
    assert!(working_sound(&data, "transport-belt").unwrap().persistent);
    assert!(working_sound(&data, "inserter").unwrap().match_progress_to_activity);
    assert_eq!(working_sound(&data, "burner-mining-drill").unwrap().sound.variations.len(), 2);
    // Utility, item, tile and entity sounds.
    let build = utility_sound(&data, "build_small").unwrap();
    assert_eq!(build.max_count, Some(3));
    assert!(item_sound(&data, "iron-plate", "pick_sound").is_some());
    assert!(walking_sound(&data, "tile", "grass-1").unwrap().variations.len() > 1);
    assert!(walking_sound(&data, "resource", "iron-ore").is_some());
    assert!(entity_sound(&data, "lab", "open_sound").is_some());
    let water = tile_ambient(&data, "water").unwrap();
    assert_eq!((water.min_entity_count, water.max_entity_count), (10, 30));
    // Every music track file exists in the install.
    let music = music_tracks(&data, "nauvis");
    assert!(music.iter().any(|t| t.track_type == "main-track"));
    for t in &music {
        assert!(t.sound.variations[0].path.exists(), "{:?}", t.sound.variations[0].path);
    }
    let planet = planet_ambience(&data, "nauvis");
    assert!(planet.base_ambience.is_some() && planet.wind.is_some());
    // Nauvis crossfades from wind (zoomed out) to base ambience (zoomed in).
    assert_eq!(planet.mix(0.3), (0.0, 1.0));
    assert_eq!(planet.mix(2.5), (1.0, 0.0));
    // Footsteps fade out when zoomed out.
    let steps = walking_sound(&data, "tile", "grass-1").unwrap();
    assert_eq!(steps.fades.at(1.0), 1.0);
    assert_eq!(steps.fades.at(0.2), 0.0);
}
