//! Bevy front end. Owns no game rules: it feeds [`PlayerInput`]s into the deterministic
//! [`Simulation`], steps it on a 60 Hz fixed schedule, and mirrors its state into sprites.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bevy::asset::io::AssetSourceBuilder;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::PrimaryWindow;
use factorio_data::GameData;
use factorio_data::sprite::{self, SpriteRef};
use factorio_sim::input::{InputAction, PlayerInput};
use factorio_sim::map::{Direction, MapPosition, SUBTILES_PER_TILE, TilePosition};
use factorio_sim::world::EntityId;
use factorio_sim::{Simulation, TICKS_PER_SECOND};

/// World units per tile. Factorio's base sprite resolution is 32 px per tile.
const TILE: f32 = 32.0;

#[derive(Resource)]
struct Sim(Simulation);

#[derive(Resource)]
struct Data(GameData);

/// Inputs gathered this frame, applied on the next fixed tick.
#[derive(Resource, Default)]
struct PendingInputs(Vec<PlayerInput>);

/// Cached sprite per prototype name (`None` if it has no sprite we can show yet).
#[derive(Resource, Default)]
struct SpriteCache(HashMap<String, Option<(Handle<Image>, SpriteRef)>>);

/// Sim entity -> Bevy entity that renders it. Render-side only; never read by the sim.
#[derive(Resource, Default)]
struct RenderMirror(HashMap<EntityId, Entity>);

#[derive(Resource)]
struct Brush(String);

#[derive(Component)]
struct HudText;

#[derive(Resource)]
struct ScreenshotRequest {
    path: PathBuf,
    frames: u32,
}

fn main() -> AppExit {
    let config = factorio_data::Config::load().unwrap_or_else(|e| fail(&e));
    let data = factorio_data::load_game_data(&config).unwrap_or_else(|e| fail(&e));
    let db = factorio_data::typed::build_prototype_db(&data).unwrap_or_else(|e| fail(&e));
    let install_root = data.install.root.to_string_lossy().into_owned();

    let mut sim = Simulation::new(Arc::new(db), 0x5EED);
    seed_demo_entities(&mut sim);

    let mut app = App::new();
    // Game assets are read straight from the user's install via `factorio://<path>`.
    app.register_asset_source("factorio", AssetSourceBuilder::platform_default(&install_root, None))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "Factorio Rewrite".into(), ..default() }),
            ..default()
        }))
        .insert_resource(Time::<Fixed>::from_hz(TICKS_PER_SECOND as f64))
        .insert_resource(ClearColor(Color::srgb(0.11, 0.12, 0.10)))
        .insert_resource(Sim(sim))
        .insert_resource(Data(data))
        .insert_resource(Brush("wooden-chest".into()))
        .init_resource::<PendingInputs>()
        .init_resource::<SpriteCache>()
        .init_resource::<RenderMirror>()
        .add_systems(Startup, setup)
        .add_systems(FixedUpdate, step_simulation)
        .add_systems(Update, (camera_controls, mouse_input, sync_render_mirror, draw_grid, update_hud, screenshot));

    if let Some(path) = std::env::var_os("FACTORIO_REWRITE_SCREENSHOT") {
        app.insert_resource(ScreenshotRequest { path: path.into(), frames: 0 });
    }
    app.run()
}

fn fail(e: &factorio_data::Error) -> ! {
    eprintln!("error: {e}");
    eprintln!("See BUILDING.md for how to point factorio-rewrite at your Factorio install.");
    std::process::exit(1);
}

/// A few entities so a fresh start shows that prototypes and sprites load.
fn seed_demo_entities(sim: &mut Simulation) {
    let names = ["wooden-chest", "iron-chest", "stone-furnace", "assembling-machine-1", "inserter", "transport-belt"];
    let mut inputs = Vec::new();
    let mut x = -9;
    for name in names {
        let Some(proto) = sim.prototypes().entities.get(name) else { continue };
        let position = proto.position_for_tile(TilePosition { x, y: -1 });
        x += proto.tile_size().0 + 1;
        inputs.push(PlayerInput {
            player: 0,
            action: InputAction::DebugPlaceEntity { prototype: name.into(), position, direction: Direction::NORTH },
        });
    }
    sim.step(&inputs);
}

fn setup(mut commands: Commands, data: Res<Data>, assets: Res<AssetServer>) {
    commands.spawn(Camera2d);

    let mods: Vec<_> = data.0.mods.iter().map(|m| format!("{} {}", m.name, m.version)).collect();
    commands.spawn((
        HudText,
        Text::new(""),
        TextFont { font_size: 16.0, ..default() },
        Node { position_type: PositionType::Absolute, top: Val::Px(8.0), left: Val::Px(8.0), ..default() },
    ));
    info!("loaded mods: {}", mods.join(", "));

    // Item icon row, straight from item prototypes.
    for (i, name) in ["iron-plate", "copper-plate", "iron-gear-wheel", "electronic-circuit", "coal"].iter().enumerate()
    {
        if let Some(icon) = sprite::item_icon(&data.0, name) {
            commands.spawn((
                sprite_for(&assets, &data.0, &icon),
                Transform::from_xyz((i as f32 - 2.0) * 1.5 * TILE, 3.0 * TILE, 0.0),
            ));
        }
    }
}

fn asset_path(data: &GameData, path: &std::path::Path) -> String {
    let rel = path.strip_prefix(&data.install.root).unwrap_or(path);
    format!("factorio://{}", rel.to_string_lossy().replace('\\', "/"))
}

fn sprite_for(assets: &AssetServer, data: &GameData, s: &SpriteRef) -> Sprite {
    let image = assets.load(asset_path(data, &s.path));
    sprite_from(image, s)
}

fn sprite_from(image: Handle<Image>, s: &SpriteRef) -> Sprite {
    let (x, y, w, h) = (s.x as f32, s.y as f32, s.width as f32, s.height as f32);
    Sprite {
        image,
        rect: Some(Rect::new(x, y, x + w, y + h)),
        custom_size: Some(Vec2::new(w, h) * s.scale as f32),
        ..default()
    }
}

fn step_simulation(mut sim: ResMut<Sim>, mut pending: ResMut<PendingInputs>) {
    let inputs = std::mem::take(&mut pending.0);
    sim.0.step(&inputs);
}

fn map_to_world(p: MapPosition) -> Vec2 {
    // Factorio's y axis points south; Bevy's points up.
    Vec2::new(p.x as f32, -(p.y as f32)) * (TILE / SUBTILES_PER_TILE as f32)
}

fn world_to_tile(v: Vec2) -> TilePosition {
    TilePosition { x: (v.x / TILE).floor() as i32, y: (-v.y / TILE).floor() as i32 }
}

fn sync_render_mirror(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    mut cache: ResMut<SpriteCache>,
    mut mirror: ResMut<RenderMirror>,
) {
    let live: std::collections::HashSet<EntityId> = sim.0.entities().map(|(id, _)| id).collect();
    mirror.0.retain(|id, e| {
        let keep = live.contains(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });

    for (id, entity) in sim.0.entities() {
        if mirror.0.contains_key(&id) {
            continue;
        }
        let sprite = cache
            .0
            .entry(entity.prototype.clone())
            .or_insert_with(|| {
                sprite::entity_sprite(&data.0, &entity.prototype)
                    .map(|s| (assets.load(asset_path(&data.0, &s.path)), s))
            })
            .clone();
        let pos = map_to_world(entity.position);
        let (bundle, shift) = match sprite {
            Some((image, s)) => (sprite_from(image, &s), Vec2::new(s.shift.0 as f32, -s.shift.1 as f32) * TILE),
            None => (Sprite::from_color(Color::srgb(0.8, 0.2, 0.8), Vec2::splat(TILE * 0.8)), Vec2::ZERO),
        };
        // Lower entities draw on top, like Factorio's south-is-closer ordering.
        let z = 1.0 - pos.y * 1e-4;
        let at = pos + shift;
        let e = commands.spawn((bundle, Transform::from_xyz(at.x, at.y, z))).id();
        mirror.0.insert(id, e);
    }
}

fn mouse_input(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform)>,
    sim: Res<Sim>,
    mut brush: ResMut<Brush>,
    mut pending: ResMut<PendingInputs>,
) {
    let brushes = ["wooden-chest", "iron-chest", "stone-furnace", "assembling-machine-1", "transport-belt"];
    for (i, key) in
        [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5].iter().enumerate()
    {
        if keys.just_pressed(*key) {
            brush.0 = brushes[i].into();
        }
    }

    let Some(cursor) = window.cursor_position() else { return };
    let (camera, transform) = *camera;
    let Ok(world) = camera.viewport_to_world_2d(transform, cursor) else { return };
    let tile = world_to_tile(world);

    if buttons.just_pressed(MouseButton::Left)
        && let Some(proto) = sim.0.prototypes().entities.get(&brush.0)
    {
        pending.0.push(PlayerInput {
            player: 0,
            action: InputAction::DebugPlaceEntity {
                prototype: brush.0.clone(),
                position: proto.position_for_tile(tile),
                direction: Direction::NORTH,
            },
        });
    }
    if buttons.just_pressed(MouseButton::Right) {
        let position = MapPosition {
            x: tile.x * SUBTILES_PER_TILE + SUBTILES_PER_TILE / 2,
            y: tile.y * SUBTILES_PER_TILE + SUBTILES_PER_TILE / 2,
        };
        pending.0.push(PlayerInput { player: 0, action: InputAction::DebugRemoveEntity { position } });
    }
}

fn camera_controls(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    camera: Single<(&mut Transform, &mut Projection), With<Camera2d>>,
) {
    let (mut transform, mut projection) = camera.into_inner();
    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        dir.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        dir.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyA) {
        dir.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        dir.x += 1.0;
    }
    let Projection::Orthographic(ortho) = projection.as_mut() else { return };
    if keys.pressed(KeyCode::KeyQ) {
        ortho.scale = (ortho.scale * (1.0 + time.delta_secs())).min(8.0);
    }
    if keys.pressed(KeyCode::KeyE) {
        ortho.scale = (ortho.scale * (1.0 - time.delta_secs())).max(0.25);
    }
    transform.translation += (dir * 20.0 * TILE * ortho.scale * time.delta_secs()).extend(0.0);
}

fn draw_grid(mut gizmos: Gizmos) {
    let color = Color::srgba(1.0, 1.0, 1.0, 0.05);
    let n = 40;
    for i in -n..=n {
        let o = i as f32 * TILE;
        let r = n as f32 * TILE;
        gizmos.line_2d(Vec2::new(o, -r), Vec2::new(o, r), color);
        gizmos.line_2d(Vec2::new(-r, o), Vec2::new(r, o), color);
    }
}

fn update_hud(
    sim: Res<Sim>,
    data: Res<Data>,
    brush: Res<Brush>,
    time: Res<Time<Real>>,
    mut text: Single<&mut Text, With<HudText>>,
    mut last: Local<(f32, u64, f64)>,
) {
    // Measured UPS over the last half second.
    let now = time.elapsed_secs();
    if now - last.0 >= 0.5 {
        last.2 = (sim.0.tick() - last.1) as f64 / (now - last.0) as f64;
        *last = (now, sim.0.tick(), last.2);
    }
    let db = sim.0.prototypes();
    text.0 = format!(
        "Factorio {}  |  mods: {}\n{} items, {} recipes, {} entity prototypes\n\
         tick {}  |  {:.0} UPS  |  checksum {:016x}  |  entities {}\n\
         brush [1-5]: {}  |  LMB place, RMB remove, WASD pan, Q/E zoom",
        data.0.install.version,
        data.0.mods.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", "),
        db.items.len(),
        db.recipes.len(),
        db.entities.len(),
        sim.0.tick(),
        last.2,
        sim.0.checksum(),
        sim.0.entities().count(),
        brush.0,
    );
}

/// `FACTORIO_REWRITE_SCREENSHOT=out.png` saves a screenshot after a few seconds and exits.
fn screenshot(mut commands: Commands, request: Option<ResMut<ScreenshotRequest>>, mut exit: MessageWriter<AppExit>) {
    let Some(mut request) = request else { return };
    request.frames += 1;
    if request.frames == 180 {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(request.path.clone()));
    }
    if request.frames == 240 {
        exit.write(AppExit::Success);
    }
}
