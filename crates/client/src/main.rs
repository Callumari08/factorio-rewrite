//! Bevy front end. Owns no game rules: it turns keyboard and mouse input into
//! [`PlayerInput`]s, steps the deterministic [`Simulation`] on a 60 Hz fixed schedule,
//! and draws its state with the game's own sprites.

// Bevy systems naturally take many parameters and complex query types.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

mod chart;
mod controls;
mod demo;
mod gui_skin;
mod render;
mod settings;
mod sound;
mod sprites;
mod terrain;
mod ui;

use std::sync::Arc;

use bevy::asset::io::AssetSourceBuilder;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use factorio_data::GameData;
use factorio_sim::input::{InputAction, PlayerInput};
use factorio_sim::map::Direction;
use factorio_sim::{Simulation, TICKS_PER_SECOND};

/// World units per tile. Factorio's base sprite resolution is 32 px per tile.
pub const TILE: f32 = 32.0;

/// The player controlled by this client.
pub const LOCAL_PLAYER: u16 = 0;

#[derive(Resource)]
pub struct Sim(pub Simulation);

#[derive(Resource, Clone)]
pub struct Data(pub Arc<GameData>);

/// Inputs gathered since the last tick, applied on the next fixed tick.
#[derive(Resource, Default)]
pub struct PendingInputs(pub Vec<InputAction>);

impl PendingInputs {
    pub fn push(&mut self, a: InputAction) {
        self.0.push(a);
    }
}

/// Build direction for the held item (the held stack itself is simulation state).
#[derive(Resource)]
pub struct Cursor {
    pub direction: Direction,
}

#[derive(Resource)]
struct ScreenshotRequest {
    path: std::path::PathBuf,
    /// Seconds of real time to wait before capturing.
    after: f32,
    taken: Option<f32>,
}

fn main() -> AppExit {
    let config = factorio_data::Config::load().unwrap_or_else(|e| fail(&e));
    let data = factorio_data::load_game_data(&config).unwrap_or_else(|e| fail(&e));
    let db = Arc::new(factorio_data::typed::build_prototype_db(&data).unwrap_or_else(|e| fail(&e)));
    let seed = std::env::var("FACTORIO_REWRITE_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0x5EED_u64);
    // Terrain from the game's noise expressions; `FACTORIO_REWRITE_SIMPLE_MAPGEN=1` uses
    // the older banded generator instead.
    let mapgen = if std::env::var_os("FACTORIO_REWRITE_SIMPLE_MAPGEN").is_some() {
        factorio_data::mapgen::default_mapgen(&db, seed)
    } else {
        factorio_data::mapgen::planet_mapgen(&data, &db, seed)
    };
    let install_root = data.install.root.to_string_lossy().into_owned();

    let mut sim = Simulation::new(db.clone(), mapgen);
    if let Some(e) = &sim.surface.noise_error {
        eprintln!("warning: the game's map generation expressions failed to compile ({e}); using a simpler generator");
    }
    let mut start = vec![PlayerInput::new(LOCAL_PLAYER, InputAction::JoinGame)];
    // Freeplay's starting inventory (base/script/freeplay/freeplay.lua, `created_items`).
    for (name, count) in [
        ("iron-plate", 8),
        ("wood", 1),
        ("pistol", 1),
        ("firearm-magazine", 10),
        ("burner-mining-drill", 1),
        ("stone-furnace", 1),
    ] {
        if let Some(item) = db.item_id(name) {
            start.push(PlayerInput::new(LOCAL_PLAYER, InputAction::CheatItems { item, count }));
        }
    }
    // `FACTORIO_REWRITE_TELEPORT=x,y` starts the character elsewhere, for screenshots.
    if let Some((x, y)) = std::env::var("FACTORIO_REWRITE_TELEPORT").ok().and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some((x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?))
    }) {
        start.push(PlayerInput::new(
            LOCAL_PLAYER,
            InputAction::CheatTeleport(factorio_sim::map::MapPosition::new(x * 256, y * 256)),
        ));
    }
    sim.step(&start);
    if std::env::var_os("FACTORIO_REWRITE_DEMO").is_some() {
        demo::build(&mut sim);
    }

    let mut app = App::new();
    // Game assets are read straight from the user's install via `factorio://<path>`.
    app.register_asset_source("factorio", AssetSourceBuilder::platform_default(&install_root, None))
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window { title: "Factorio Rewrite".into(), ..default() }),
                    ..default()
                })
                .set(ImagePlugin::default_linear()),
        )
        .insert_resource(Time::<Fixed>::from_hz(TICKS_PER_SECOND as f64))
        .insert_resource(ClearColor(Color::srgb(0.05, 0.05, 0.05)))
        .insert_resource(Sim(sim))
        .insert_resource(Data(Arc::new(data)))
        .insert_resource(Cursor { direction: Direction::NORTH })
        .init_resource::<PendingInputs>()
        .add_plugins((
            chart::ChartPlugin,
            sprites::SpritesPlugin,
            terrain::TerrainPlugin,
            render::RenderPlugin,
            gui_skin::GuiSkinPlugin,
            ui::UiPlugin,
            controls::ControlsPlugin,
            sound::SoundPlugin,
            settings::SettingsPlugin,
        ))
        .add_systems(Startup, setup_camera)
        .add_systems(FixedUpdate, step_simulation)
        .add_systems(Update, (screenshot, zoom_sweep));

    if let Some(path) = std::env::var_os("FACTORIO_REWRITE_SCREENSHOT") {
        let after = std::env::var("FACTORIO_REWRITE_SCREENSHOT_AFTER").ok().and_then(|s| s.parse().ok()).unwrap_or(4.0);
        app.insert_resource(ScreenshotRequest { path: path.into(), after, taken: None });
    }
    app.run()
}

fn fail(e: &factorio_data::Error) -> ! {
    eprintln!("error: {e}");
    eprintln!("See BUILDING.md for how to point factorio-rewrite at your Factorio install.");
    std::process::exit(1);
}

fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        // `FACTORIO_REWRITE_ZOOM` sets the starting camera scale (bigger is further out).
        Projection::Orthographic(OrthographicProjection {
            scale: std::env::var("FACTORIO_REWRITE_ZOOM").ok().and_then(|z| z.parse().ok()).unwrap_or(1.0),
            ..OrthographicProjection::default_2d()
        }),
    ));
}

fn step_simulation(mut sim: ResMut<Sim>, mut pending: ResMut<PendingInputs>, mut events: ResMut<sound::SimEvents>) {
    let inputs: Vec<PlayerInput> = pending.0.drain(..).map(|a| PlayerInput::new(LOCAL_PLAYER, a)).collect();
    sim.0.step(&inputs);
    events.0.extend(sim.0.events().iter().cloned());
}

/// `FACTORIO_REWRITE_SCREENSHOT=out.png` saves a screenshot after
/// `FACTORIO_REWRITE_SCREENSHOT_AFTER` seconds (default 4) and exits.
fn screenshot(
    mut commands: Commands,
    request: Option<ResMut<ScreenshotRequest>>,
    time: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut request) = request else { return };
    let now = time.elapsed_secs();
    match request.taken {
        None if now >= request.after => {
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(request.path.clone()));
            request.taken = Some(now);
        }
        Some(t) if now >= t + 1.0 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

/// `FACTORIO_REWRITE_ZOOM_SWEEP=dir` (after six seconds to load) zooms the camera from 1
/// out to 6 and back over 12 seconds, saving a screenshot to `dir` every half second, then
/// exits. For checking that the ground's level of detail switches without popping.
fn zoom_sweep(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut cam: Query<&mut Projection, With<Camera2d>>,
    mut last: Local<Option<i32>>,
    mut stats: Local<(i32, u32, f32, f32)>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(dir) = std::env::var_os("FACTORIO_REWRITE_ZOOM_SWEEP") else { return };
    let t = time.elapsed_secs() - 6.0;
    if t < 0.0 {
        return;
    }
    // Frame times per second of the sweep: frames, total and worst ms.
    let ms = time.delta_secs() * 1000.0;
    let second = t as i32;
    if stats.0 != second {
        if stats.1 > 0 {
            info!(
                "sweep second {}: {} frames, mean {:.2} ms, worst {:.2} ms",
                stats.0,
                stats.1,
                stats.2 / stats.1 as f32,
                stats.3
            );
        }
        *stats = (second, 0, 0.0, 0.0);
    }
    stats.1 += 1;
    stats.2 += ms;
    stats.3 = stats.3.max(ms);
    let scale = if t < 6.0 { 1.0 + 5.0 * t / 6.0 } else { (6.0 - 5.0 * (t - 6.0) / 6.0).max(1.0) };
    if let Ok(mut p) = cam.single_mut()
        && let Projection::Orthographic(o) = p.as_mut()
    {
        o.scale = scale;
    }
    let frame = (t / 0.5) as i32;
    if *last != Some(frame) && t <= 12.5 {
        *last = Some(frame);
        let path = std::path::Path::new(&dir).join(format!("sweep{frame:02}.png"));
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
    if t > 13.5 {
        exit.write(AppExit::Success);
    }
}

/// Map position (1/256 tiles, y down) to Bevy world coordinates (y up).
pub fn map_to_world(p: factorio_sim::map::MapPosition) -> Vec2 {
    Vec2::new(p.x as f32, -(p.y as f32)) * (TILE / 256.0)
}

pub fn world_to_map(v: Vec2) -> factorio_sim::map::MapPosition {
    factorio_sim::map::MapPosition::new((v.x / TILE * 256.0).floor() as i32, (-v.y / TILE * 256.0).floor() as i32)
}

/// Draw order: things further south are drawn on top.
pub fn depth(world_y: f32, layer: f32) -> f32 {
    layer + (-world_y / TILE) * 0.0001
}
