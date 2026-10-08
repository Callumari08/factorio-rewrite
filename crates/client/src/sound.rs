//! Game sounds from the player's install: machine working loops, building and mining,
//! hand crafting, research, footsteps, GUI clicks, water and wind ambience, and music.
//!
//! Volumes come from the player's own Factorio settings (`config.ini`). Positional sounds
//! fade with distance from the centre of the screen and pan left/right. The exact falloff
//! curve of the game is not documented; ours is `(1 - d / R)²` with `R` = 35 tiles times
//! the sound's `audible_distance_modifier`, and quieter when zoomed out.

use std::collections::HashMap;

use bevy::audio::{PlaybackMode, SpatialListener, SpatialScale, Volume};
use bevy::prelude::*;
use factorio_data::sound::{MusicTrack, Sound, SoundSettings, WorkingSound, WorldAmbient};
use factorio_sim::map::{MapPosition, SUBTILES_PER_TILE};
use factorio_sim::proto::EntityProtoId;
use factorio_sim::world::{EntityId, EntityState, GameEvent};

use crate::{Data, LOCAL_PLAYER, Sim, TILE};

/// Base hearing distance in tiles, scaled by each sound's `audible_distance_modifier`.
const HEARING_TILES: f32 = 35.0;
/// Distance between the listener's ears in world units: sources further than half of this
/// to the side are fully panned.
const EAR_GAP: f32 = TILE * 20.0;
/// At most this many machines of one prototype play their working sound at once.
const DEFAULT_MAX_PER_PROTOTYPE: usize = 6;

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SimEvents>()
            .init_resource::<GuiClicks>()
            .init_resource::<Listener>()
            .init_resource::<Bank>()
            .init_resource::<Loops>()
            .init_resource::<Rng>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    listener,
                    event_sounds,
                    working_sounds,
                    footsteps,
                    mining_sounds,
                    ambience,
                    music,
                    cursor_sounds,
                    gui_clicks,
                )
                    .chain(),
            );
    }
}

/// Simulation events gathered from every fixed tick since the last frame.
#[derive(Resource, Default)]
pub struct SimEvents(pub Vec<GameEvent>);

#[derive(Resource)]
struct Settings(SoundSettings);

/// Parsed sounds, looked up once per key.
#[derive(Resource, Default)]
struct Bank {
    sounds: HashMap<String, Option<Sound>>,
    working: HashMap<EntityProtoId, Option<WorkingSound>>,
    ambient: HashMap<factorio_sim::proto::TileId, Option<WorldAmbient>>,
}

impl Bank {
    fn sound(&mut self, key: &str, f: impl FnOnce() -> Option<Sound>) -> Option<Sound> {
        self.sounds.entry(key.to_owned()).or_insert_with(f).clone()
    }

    fn utility(&mut self, data: &Data, key: &str) -> Option<Sound> {
        let d = data.0.clone();
        self.sound(&format!("utility:{key}"), || factorio_data::sound::utility_sound(&d, key))
    }
}

/// Client-side randomness for picking variations; not part of the simulation.
#[derive(Resource)]
struct Rng(u64);

impl Default for Rng {
    fn default() -> Self {
        Rng(0x9E37_79B9_7F4A_7C15)
    }
}

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, (lo, hi): (f32, f32)) -> f32 {
        lo + (hi - lo) * self.next()
    }
    fn index(&mut self, n: usize) -> usize {
        ((self.next() * n as f32) as usize).min(n.saturating_sub(1))
    }
}

/// Where the player hears from: the camera centre, in tiles, and the zoom.
#[derive(Resource, Default, Clone, Copy)]
struct Listener {
    tiles: Vec2,
    zoom: f32,
    z: f32,
}

/// A looping sound attached to a machine (or shared by a prototype), fading in and out.
struct Loop {
    entity: Entity,
    volume: f32,
    target: f32,
    fade_in: f32,
    fade_out: f32,
}

#[derive(Resource, Default)]
struct Loops {
    machines: HashMap<EntityId, Loop>,
    persistent: HashMap<EntityProtoId, Loop>,
    /// Inserters holding an item last frame, to play their swing sound once per pickup.
    inserter_hands: HashMap<EntityId, bool>,
    ambience: Vec<Entity>,
    last_cursor: Option<factorio_sim::inventory::ItemStack>,
    last_opened: Option<(EntityId, EntityProtoId)>,
    footstep_timer: f32,
    mining_timer: f32,
    water_timer: f32,
    music: Option<Entity>,
    music_gap: f32,
    music_order: Vec<usize>,
    music_next: usize,
    tracks: Vec<MusicTrack>,
}

fn setup(mut commands: Commands, data: Res<Data>) {
    commands.insert_resource(Settings(SoundSettings::load(&data.0.install.root)));
}

fn listener(
    mut commands: Commands,
    camera: Single<(Entity, &Transform, &Projection, Option<&SpatialListener>), With<Camera2d>>,
    mut l: ResMut<Listener>,
) {
    let (entity, t, proj, has) = *camera;
    if has.is_none() {
        commands.entity(entity).insert(SpatialListener::new(EAR_GAP));
    }
    l.tiles = Vec2::new(t.translation.x, t.translation.y) / TILE;
    l.z = t.translation.z;
    l.zoom = match proj {
        Projection::Orthographic(o) => o.scale,
        _ => 1.0,
    };
}

/// Tiles in Bevy's orientation (y up) for a map position.
fn tiles(p: MapPosition) -> Vec2 {
    Vec2::new(p.x as f32, -(p.y as f32)) / SUBTILES_PER_TILE as f32
}

impl Listener {
    /// Distance gain for a sound at `at` (tiles), 0 when out of hearing range.
    fn gain(&self, at: Vec2, sound: &Sound) -> f32 {
        let zoom = self.zoom.max(1.0);
        let range = HEARING_TILES * sound.audible_distance_modifier.max(0.05) * zoom;
        let d = at.distance(self.tiles);
        let g = (1.0 - d / range).max(0.0);
        g * g / zoom
    }

    /// Playback settings for a positional one-shot. Positions are scaled down so the
    /// audio backend's own distance attenuation never applies (ours does); only its
    /// left/right panning is used.
    fn spatial(&self) -> PlaybackSettings {
        let scale = 1.0 / (TILE * HEARING_TILES * 2.5 * self.zoom.max(1.0));
        PlaybackSettings { spatial: true, spatial_scale: Some(SpatialScale::new(scale)), ..PlaybackSettings::DESPAWN }
    }
}

/// Starts a sound. `at` is a world position in tiles for positional sounds.
fn play(
    commands: &mut Commands,
    assets: &AssetServer,
    data: &Data,
    rng: &mut Rng,
    settings: &SoundSettings,
    listener: &Listener,
    sound: &Sound,
    category: &str,
    at: Option<Vec2>,
) {
    let gain = at.map_or(1.0, |p| listener.gain(p, sound));
    let category = sound.category.as_deref().unwrap_or(category);
    let file = &sound.variations[rng.index(sound.variations.len())];
    let volume = rng.range(file.volume) * settings.volume(category) * gain;
    if volume <= 0.001 {
        return;
    }
    if std::env::var_os("FACTORIO_REWRITE_SOUND_LOG").is_some() {
        info!("sound {} volume {volume:.3} at {at:?}", file.path.display());
    }
    let handle: Handle<AudioSource> = assets.load(crate::sprites::asset_path(data, &file.path));
    let mut playback = PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)).with_speed(rng.range(file.speed));
    let mut transform = Transform::default();
    if let Some(p) = at {
        playback = listener.spatial().with_volume(Volume::Linear(volume)).with_speed(rng.range(file.speed));
        transform = Transform::from_xyz(p.x * TILE, p.y * TILE, listener.z);
    }
    commands.spawn((AudioPlayer::new(handle), playback, transform));
}

fn size_word(db: &factorio_sim::PrototypeDb, e: EntityProtoId) -> &'static str {
    let p = db.entity(e);
    match p.tile_width * p.tile_height {
        0..=1 => "small",
        2..=4 => "medium",
        5..=16 => "large",
        _ => "huge",
    }
}

fn event_sounds(
    mut commands: Commands,
    assets: Res<AssetServer>,
    data: Res<Data>,
    sim: Res<Sim>,
    settings: Res<Settings>,
    listener: Res<Listener>,
    mut events: ResMut<SimEvents>,
    mut bank: ResMut<Bank>,
    mut rng: ResMut<Rng>,
) {
    let db = sim.0.prototypes();
    for e in events.0.drain(..) {
        let (sound, category, at) = match e {
            GameEvent::Built { entity, position } => {
                (bank.utility(&data, &format!("build_{}", size_word(db, entity))), "game-effect", Some(tiles(position)))
            }
            GameEvent::Mined { entity, position } => {
                let d = data.0.clone();
                let name = db.entity(entity).name.clone();
                let own = bank
                    .sound(&format!("mined:{name}"), || factorio_data::sound::entity_sound(&d, &name, "mined_sound"));
                let s = own.or_else(|| bank.utility(&data, &format!("deconstruct_{}", size_word(db, entity))));
                (s, "game-effect", Some(tiles(position)))
            }
            GameEvent::Crafted { player, .. } if player == LOCAL_PLAYER => {
                (bank.utility(&data, "crafting_finished"), "gui-effect", None)
            }
            GameEvent::ResearchFinished(_) => (bank.utility(&data, "research_completed"), "alert", None),
            _ => (None, "", None),
        };
        if let Some(s) = sound {
            play(&mut commands, &assets, &data, &mut rng, &settings.0, &listener, &s, category, at);
        }
    }
}

/// Whether a machine is doing work now, for its working sound.
fn working(state: &EntityState) -> bool {
    match state {
        EntityState::Drill(d) => d.working,
        EntityState::Crafter(c) => c.crafting,
        EntityState::Lab(l) => l.working,
        EntityState::Fluid(f) => f.last_power.is_positive(),
        EntityState::Belt => true,
        _ => false,
    }
}

fn working_sounds(
    mut commands: Commands,
    assets: Res<AssetServer>,
    data: Res<Data>,
    sim: Res<Sim>,
    time: Res<Time>,
    settings: Res<Settings>,
    listener: Res<Listener>,
    mut bank: ResMut<Bank>,
    mut loops: ResMut<Loops>,
    mut rng: ResMut<Rng>,
    mut sinks: Query<&mut AudioSink>,
    mut spatial_sinks: Query<&mut bevy::audio::SpatialAudioSink>,
) {
    let db = sim.0.prototypes();
    let reach = (HEARING_TILES * listener.zoom.max(1.0)) as i32 + 2;
    let centre = MapPosition::new(
        (listener.tiles.x * SUBTILES_PER_TILE as f32) as i32,
        (-listener.tiles.y * SUBTILES_PER_TILE as f32) as i32,
    );
    let area = factorio_sim::map::Area {
        left_top: centre.offset(-reach * SUBTILES_PER_TILE, -reach * SUBTILES_PER_TILE),
        right_bottom: centre.offset(reach * SUBTILES_PER_TILE, reach * SUBTILES_PER_TILE),
    };
    let ids = sim.0.entities_in(area);

    // Candidates per prototype, nearest first.
    let mut by_proto: HashMap<EntityProtoId, Vec<(f32, EntityId, Vec2)>> = HashMap::new();
    let mut persistent_gain: HashMap<EntityProtoId, f32> = HashMap::new();
    let mut hands: HashMap<EntityId, bool> = HashMap::new();
    for id in ids {
        let Some(e) = sim.0.entity(id) else { continue };
        let d = data.0.clone();
        let name = db.entity(e.proto).name.clone();
        let Some(ws) =
            bank.working.entry(e.proto).or_insert_with(|| factorio_data::sound::working_sound(&d, &name)).clone()
        else {
            continue;
        };
        let at = tiles(e.position);
        if let EntityState::Inserter(i) = &e.state {
            // One swing sound per pickup.
            let holding = i.hand.is_some();
            hands.insert(id, holding);
            if holding && loops.inserter_hands.get(&id) == Some(&false) {
                play(
                    &mut commands,
                    &assets,
                    &data,
                    &mut rng,
                    &settings.0,
                    &listener,
                    &ws.sound,
                    "game-effect",
                    Some(at),
                );
            }
            continue;
        }
        if !working(&e.state) {
            continue;
        }
        let gain = listener.gain(at, &ws.sound);
        if gain <= 0.0 {
            continue;
        }
        if ws.persistent {
            *persistent_gain.entry(e.proto).or_default() += gain;
        } else {
            by_proto.entry(e.proto).or_default().push((gain, id, at));
        }
    }
    loops.inserter_hands = hands;

    let fade = |ticks: u32| if ticks == 0 { f32::INFINITY } else { 60.0 / ticks as f32 };
    let mut wanted: HashMap<EntityId, (f32, Vec2, WorkingSound)> = HashMap::new();
    for (proto, mut list) in by_proto {
        let ws = bank.working[&proto].clone().unwrap();
        list.sort_by(|a, b| b.0.total_cmp(&a.0));
        let max = ws.max_sounds_per_prototype.map_or(DEFAULT_MAX_PER_PROTOTYPE, |m| m as usize);
        for (gain, id, at) in list.into_iter().take(max) {
            wanted.insert(id, (gain, at, ws.clone()));
        }
    }
    let effects = settings.0.volume("game-effect");
    // Start and retarget machine loops.
    for (id, (gain, at, ws)) in &wanted {
        let file = &ws.sound.variations[0];
        let target = file.volume.1 * effects * gain;
        if let Some(l) = loops.machines.get_mut(id) {
            l.target = target;
            continue;
        }
        if std::env::var_os("FACTORIO_REWRITE_SOUND_LOG").is_some() {
            info!("loop {} target {target:.3}", file.path.display());
        }
        let handle: Handle<AudioSource> = assets.load(crate::sprites::asset_path(&data, &file.path));
        let playback = PlaybackSettings { mode: PlaybackMode::Loop, ..listener.spatial() }
            .with_volume(Volume::Linear(0.0))
            .with_speed(file.speed.1);
        let entity = commands
            .spawn((AudioPlayer::new(handle), playback, Transform::from_xyz(at.x * TILE, at.y * TILE, listener.z)))
            .id();
        loops.machines.insert(
            *id,
            Loop { entity, volume: 0.0, target, fade_in: fade(ws.fade_in_ticks), fade_out: fade(ws.fade_out_ticks) },
        );
    }
    for (id, l) in loops.machines.iter_mut() {
        if !wanted.contains_key(id) {
            l.target = 0.0;
        }
    }
    // Shared loops (belts): one per prototype, louder the more of it is nearby.
    for (proto, gain) in &persistent_gain {
        let ws = bank.working[proto].clone().unwrap();
        let file = &ws.sound.variations[0];
        let target = file.volume.1 * effects * gain.min(1.0);
        if let Some(l) = loops.persistent.get_mut(proto) {
            l.target = target;
            continue;
        }
        let handle: Handle<AudioSource> = assets.load(crate::sprites::asset_path(&data, &file.path));
        let entity =
            commands.spawn((AudioPlayer::new(handle), PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)))).id();
        loops.persistent.insert(*proto, Loop { entity, volume: 0.0, target, fade_in: 4.0, fade_out: 4.0 });
    }
    for (proto, l) in loops.persistent.iter_mut() {
        if !persistent_gain.contains_key(proto) {
            l.target = 0.0;
        }
    }

    // Fade towards the targets; stop silent loops.
    let dt = time.delta_secs();
    let mut set = |l: &mut Loop| -> bool {
        let step = if l.target > l.volume { l.fade_in } else { l.fade_out } * dt * l.target.max(l.volume).max(0.05);
        l.volume = if l.target > l.volume { (l.volume + step).min(l.target) } else { (l.volume - step).max(l.target) };
        if let Ok(mut s) = sinks.get_mut(l.entity) {
            s.set_volume(Volume::Linear(l.volume));
        } else if let Ok(mut s) = spatial_sinks.get_mut(l.entity) {
            s.set_volume(Volume::Linear(l.volume));
        }
        l.target <= 0.0 && l.volume <= 0.0
    };
    let mut done = Vec::new();
    for (id, l) in loops.machines.iter_mut() {
        if set(l) {
            done.push((*id, l.entity));
        }
    }
    for (id, entity) in done {
        loops.machines.remove(&id);
        commands.entity(entity).despawn();
    }
    for l in loops.persistent.values_mut() {
        set(l);
    }
}

/// Footsteps on the tile (or resource) under the character while walking.
fn footsteps(
    mut commands: Commands,
    assets: Res<AssetServer>,
    data: Res<Data>,
    sim: Res<Sim>,
    time: Res<Time>,
    settings: Res<Settings>,
    listener: Res<Listener>,
    mut bank: ResMut<Bank>,
    mut loops: ResMut<Loops>,
    mut rng: ResMut<Rng>,
) {
    let Some(c) = sim.0.player(LOCAL_PLAYER).and_then(|p| p.character.as_ref()) else { return };
    if c.walking.is_none() {
        loops.footstep_timer = 0.0;
        return;
    }
    loops.footstep_timer -= time.delta_secs();
    if loops.footstep_timer > 0.0 {
        return;
    }
    // Two steps per run cycle (running_sound_animation_positions).
    loops.footstep_timer = 0.3;
    let db = sim.0.prototypes();
    let t = c.position().tile();
    let d = data.0.clone();
    let sound = match sim.0.surface.resource(t) {
        Some(r) => {
            let name = db.entity(r.proto).name.clone();
            bank.sound(&format!("walk:resource:{name}"), || factorio_data::sound::walking_sound(&d, "resource", &name))
        }
        None => sim.0.surface.tile(t).and_then(|tile| {
            let name = db.tile(tile).name.clone();
            bank.sound(&format!("walk:tile:{name}"), || factorio_data::sound::walking_sound(&d, "tile", &name))
        }),
    };
    if let Some(s) = sound {
        play(&mut commands, &assets, &data, &mut rng, &settings.0, &listener, &s, "walking", None);
    }
}

/// The pickaxe while mining by hand.
fn mining_sounds(
    mut commands: Commands,
    assets: Res<AssetServer>,
    data: Res<Data>,
    sim: Res<Sim>,
    time: Res<Time>,
    settings: Res<Settings>,
    listener: Res<Listener>,
    mut bank: ResMut<Bank>,
    mut loops: ResMut<Loops>,
    mut rng: ResMut<Rng>,
) {
    let Some(c) = sim.0.player(LOCAL_PLAYER).and_then(|p| p.character.as_ref()) else { return };
    let Some(m) = &c.mining else {
        loops.mining_timer = 0.0;
        return;
    };
    loops.mining_timer -= time.delta_secs();
    if loops.mining_timer > 0.0 {
        return;
    }
    loops.mining_timer = 0.55;
    let stone = match &m.target {
        factorio_sim::player::MiningTarget::Resource(t) => {
            sim.0.surface.resource(*t).is_some_and(|r| sim.0.prototypes().entity(r.proto).name.contains("stone"))
        }
        _ => false,
    };
    let key = if stone { "axe_mining_stone" } else { "axe_mining_ore" };
    if let Some(s) = bank.utility(&data, key) {
        play(&mut commands, &assets, &data, &mut rng, &settings.0, &listener, &s, "game-effect", None);
    }
}

/// Wind and base ambience loops, and water lapping near water.
fn ambience(
    mut commands: Commands,
    assets: Res<AssetServer>,
    data: Res<Data>,
    sim: Res<Sim>,
    time: Res<Time>,
    settings: Res<Settings>,
    listener: Res<Listener>,
    mut bank: ResMut<Bank>,
    mut loops: ResMut<Loops>,
    mut rng: ResMut<Rng>,
) {
    if loops.ambience.is_empty() {
        let (base, wind) = factorio_data::sound::planet_ambience(&data.0, "nauvis");
        // The game crossfades these with zoom; at normal zoom both are heard.
        for (sound, category, share) in [(base, "world-ambient", 1.0), (wind, "wind", 0.5)] {
            let Some(s) = sound else { continue };
            let file = &s.variations[0];
            let handle: Handle<AudioSource> = assets.load(crate::sprites::asset_path(&data, &file.path));
            let volume = file.volume.1 * settings.0.volume(category) * share;
            let e =
                commands.spawn((AudioPlayer::new(handle), PlaybackSettings::LOOP.with_volume(Volume::Linear(volume))));
            loops.ambience.push(e.id());
        }
        if loops.ambience.is_empty() {
            loops.ambience.push(Entity::PLACEHOLDER);
        }
    }

    loops.water_timer -= time.delta_secs();
    if loops.water_timer > 0.0 {
        return;
    }
    loops.water_timer = 0.5;
    // Count tiles with ambient sounds around the listener and play one now and then.
    let db = sim.0.prototypes();
    let centre = MapPosition::new(
        (listener.tiles.x * SUBTILES_PER_TILE as f32) as i32,
        (-listener.tiles.y * SUBTILES_PER_TILE as f32) as i32,
    )
    .tile();
    let mut found: HashMap<factorio_sim::proto::TileId, Vec<Vec2>> = HashMap::new();
    let r = 15;
    for y in centre.y - r..=centre.y + r {
        for x in centre.x - r..=centre.x + r {
            let t = factorio_sim::map::TilePosition::new(x, y);
            let Some(tile) = sim.0.surface.tile(t) else { continue };
            let d = data.0.clone();
            let name = db.tile(tile).name.clone();
            if bank.ambient.entry(tile).or_insert_with(|| factorio_data::sound::tile_ambient(&d, &name)).is_some() {
                found.entry(tile).or_default().push(Vec2::new(x as f32 + 0.5, -(y as f32 + 0.5)));
            }
        }
    }
    for (tile, spots) in found {
        let a = bank.ambient[&tile].clone().unwrap();
        let sounds = (spots.len() as f32 * a.entity_to_sound_ratio) as u32;
        if sounds < a.min_entity_count {
            continue;
        }
        let sounds = sounds.min(a.max_entity_count) as f32;
        // On average `sounds` instances each pausing `average_pause_seconds` between plays.
        let chance = 0.5 * sounds / (a.average_pause_seconds.max(0.5) * 10.0);
        if rng.next() < chance {
            let at = spots[rng.index(spots.len())];
            play(&mut commands, &assets, &data, &mut rng, &settings.0, &listener, &a.sound, "environment", Some(at));
        }
    }
}

/// The planet's music, shuffled, with a pause between tracks.
fn music(
    mut commands: Commands,
    assets: Res<AssetServer>,
    data: Res<Data>,
    time: Res<Time>,
    settings: Res<Settings>,
    mut loops: ResMut<Loops>,
    mut rng: ResMut<Rng>,
    playing: Query<(), With<AudioPlayer>>,
) {
    if settings.0.volume("music") <= 0.0 {
        return;
    }
    if loops.tracks.is_empty() {
        loops.tracks = factorio_data::sound::music_tracks(&data.0, "nauvis")
            .into_iter()
            .filter(|t| t.track_type != "menu-track")
            .collect();
        let n = loops.tracks.len();
        let mut order: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            order.swap(i, rng.index(i + 1));
        }
        loops.music_order = order;
        loops.music_gap = 3.0;
        if n == 0 {
            return;
        }
    }
    if let Some(e) = loops.music {
        if playing.get(e).is_ok() {
            return;
        }
        loops.music = None;
        loops.music_gap = 20.0 + 40.0 * rng.next();
    }
    loops.music_gap -= time.delta_secs();
    if loops.music_gap > 0.0 || loops.music_order.is_empty() {
        return;
    }
    let i = loops.music_order[loops.music_next % loops.music_order.len()];
    loops.music_next += 1;
    let track = &loops.tracks[i];
    let file = &track.sound.variations[0];
    let handle: Handle<AudioSource> = assets.load(crate::sprites::asset_path(&data, &file.path));
    let volume = file.volume.1 * settings.0.volume("music");
    let e = commands.spawn((AudioPlayer::new(handle), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume))));
    loops.music = Some(e.id());
}

/// Picking up and putting down stacks, and opening and closing machine windows.
fn cursor_sounds(
    mut commands: Commands,
    assets: Res<AssetServer>,
    data: Res<Data>,
    sim: Res<Sim>,
    settings: Res<Settings>,
    listener: Res<Listener>,
    mut bank: ResMut<Bank>,
    mut loops: ResMut<Loops>,
    mut rng: ResMut<Rng>,
) {
    let Some(p) = sim.0.player(LOCAL_PLAYER) else { return };
    let db = sim.0.prototypes();
    let cursor = p.character.as_ref().and_then(|c| c.cursor);
    let item_sound = |bank: &mut Bank, item: factorio_sim::proto::ItemId, key: &str| {
        let d = data.0.clone();
        let name = db.item(item).name.clone();
        let own = bank.sound(&format!("{key}:{name}"), || factorio_data::sound::item_sound(&d, &name, key));
        own.or_else(|| bank.utility(&data, "inventory_move"))
    };
    let sound = match (loops.last_cursor, cursor) {
        (None, Some(s)) => item_sound(&mut bank, s.item, "pick_sound"),
        (Some(s), None) => item_sound(&mut bank, s.item, "drop_sound"),
        (Some(a), Some(b)) if a != b => item_sound(&mut bank, b.item, "inventory_move_sound"),
        _ => None,
    };
    loops.last_cursor = cursor;
    if let Some(s) = sound {
        play(&mut commands, &assets, &data, &mut rng, &settings.0, &listener, &s, "gui-effect", None);
    }

    let opened = p.opened.and_then(|id| sim.0.entity(id).map(|e| (id, e.proto)));
    if opened != loops.last_opened {
        let d = data.0.clone();
        let mut sounds = Vec::new();
        if let Some((_, proto)) = loops.last_opened {
            let name = db.entity(proto).name.clone();
            sounds.push(
                bank.sound(&format!("close:{name}"), || factorio_data::sound::entity_sound(&d, &name, "close_sound")),
            );
        }
        if let Some((_, proto)) = opened {
            let name = db.entity(proto).name.clone();
            sounds.push(
                bank.sound(&format!("open:{name}"), || factorio_data::sound::entity_sound(&d, &name, "open_sound")),
            );
        }
        for s in sounds.into_iter().flatten() {
            play(&mut commands, &assets, &data, &mut rng, &settings.0, &listener, &s, "gui-effect", None);
        }
        loops.last_opened = opened;
    }
}

/// GUI button clicks since the last frame (inventory slots make item sounds instead).
#[derive(Resource, Default)]
pub struct GuiClicks(pub u32);

fn gui_clicks(
    mut commands: Commands,
    assets: Res<AssetServer>,
    data: Res<Data>,
    settings: Res<Settings>,
    listener: Res<Listener>,
    mut clicks: ResMut<GuiClicks>,
    mut bank: ResMut<Bank>,
    mut rng: ResMut<Rng>,
) {
    if clicks.0 == 0 {
        return;
    }
    clicks.0 = 0;
    if let Some(s) = bank.utility(&data, "gui_click") {
        play(&mut commands, &assets, &data, &mut rng, &settings.0, &listener, &s, "gui-effect", None);
    }
}
