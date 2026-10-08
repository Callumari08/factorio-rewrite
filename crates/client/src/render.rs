//! Draws simulation state: entities, belt contents, ground items, the character and the
//! build preview. Purely a view; nothing here changes the simulation.

use std::collections::{HashMap, HashSet};
use std::f32::consts::TAU;

use bevy::prelude::*;
use factorio_data::sprite::LayerKind;
use factorio_sim::belt::{BeltKind, BeltShape};
use factorio_sim::map::Direction;
use factorio_sim::proto::EntityData;
use factorio_sim::world::{EntityId, EntityState};

use crate::sprites::{Loaded, Sprites};
use crate::{Cursor, Data, LOCAL_PLAYER, Sim, TILE, depth, map_to_world};

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mirror>()
            .init_resource::<Pools>()
            .add_systems(Startup, spawn_character)
            .add_systems(Update, (sync_entities, draw_items, draw_character, draw_ghost, follow_camera).chain());
    }
}

struct Mirrored {
    layers: Vec<Entity>,
    hand: Option<Entity>,
    key: String,
}

#[derive(Resource, Default)]
struct Mirror(HashMap<EntityId, Mirrored>);

#[derive(Resource, Default)]
struct Pools {
    items: Vec<Entity>,
}

#[derive(Component)]
struct CharacterSprite;

#[derive(Component)]
pub struct Ghost;

fn dir_index(d: Direction) -> usize {
    d.cardinal_index()
}

fn dir_name(d: Direction) -> &'static str {
    ["north", "east", "south", "west"][d.cardinal_index()]
}

/// Row of the belt sheet for a belt's direction and shape. Rows are named after the side
/// items enter from: a belt facing north fed from the west (a left turn) is `west_to_north`.
pub fn belt_row_name(direction: Direction, shape: BeltShape) -> String {
    let from = match shape {
        BeltShape::Straight => return format!("{}_index", dir_name(direction)),
        BeltShape::CurveLeft => direction.rotate_ccw(),
        BeltShape::CurveRight => direction.rotate_cw(),
    };
    format!("{}_to_{}_index", dir_name(from), dir_name(direction))
}

type Look = (String, Vec<(Loaded, LayerKind)>);

/// Whether a machine is doing work right now (drives its animations).
fn is_working(state: &EntityState) -> bool {
    match state {
        EntityState::Drill(d) => d.working,
        EntityState::Crafter(c) => c.crafting,
        EntityState::Fluid(f) => f.last_power.is_positive(),
        EntityState::Lab(l) => l.working,
        _ => false,
    }
}

/// Sprite layers for an entity in its current state, with a key that changes whenever
/// the picture does.
fn entity_look(sim: &Sim, data: &Data, sprites: &mut Sprites, assets: &AssetServer, id: EntityId) -> Option<Look> {
    let single = |l: Option<(String, Loaded)>| l.map(|(k, l)| (k, vec![(l, LayerKind::Normal)]));
    let e = sim.0.entity(id)?;
    let is_belt_or_pipe = matches!(e.state, EntityState::Belt) || sim.0.prototypes().entity(e.proto).kind == "pipe";
    if is_belt_or_pipe {
        return single(entity_look_single(sim, data, sprites, assets, id));
    }
    let name = sim.0.prototypes().entity(e.proto).name.clone();
    let di = dir_index(e.direction);
    let working = is_working(&e.state);
    let t = if working { sim.0.tick() } else { 0 };
    let key = format!("layers:{name}:{di}:{working}:{t}");
    let layers: Vec<(Loaded, LayerKind)> = factorio_data::sprite::entity_layers(&data.0, &name, di, t, working)
        .into_iter()
        .map(|(sprite, kind)| {
            (Loaded { image: assets.load(crate::sprites::asset_path(data, &sprite.path)), sprite }, kind)
        })
        .collect();
    if layers.is_empty() {
        return single(entity_look_single(sim, data, sprites, assets, id));
    }
    Some((key, layers))
}

/// Single-sprite lookup for belts, pipes and anything without layered graphics.
fn entity_look_single(
    sim: &Sim,
    data: &Data,
    sprites: &mut Sprites,
    assets: &AssetServer,
    id: EntityId,
) -> Option<(String, Loaded)> {
    let e = sim.0.entity(id)?;
    let db = sim.0.prototypes();
    let proto = db.entity(e.proto);
    let name = proto.name.clone();
    let d = data.0.clone();
    let tick = sim.0.tick();
    match (&e.state, &proto.data) {
        (EntityState::Belt, _) => {
            let b = sim.0.belts.get(id)?;
            match b.kind {
                BeltKind::Belt => {
                    let index = belt_row_name(b.direction, b.shape);
                    let frame = ((tick as i64 * b.speed as i64 / 16) % 16) as u32;
                    let key = format!("belt:{name}:{index}:{frame}");
                    let l = sprites.get(assets, data, &key, || {
                        factorio_data::sprite::belt_sheet(&d, &name).map(|s| s.frame(&index, frame))
                    })?;
                    Some((key, l))
                }
                BeltKind::UndergroundInput | BeltKind::UndergroundOutput => {
                    let input = b.kind == BeltKind::UndergroundInput;
                    let di = dir_index(if input { b.direction } else { b.direction.opposite() });
                    let key = format!("ug:{name}:{di}:{input}");
                    let l = sprites
                        .get(assets, data, &key, || factorio_data::sprite::underground_sprite(&d, &name, di, input))?;
                    Some((key, l))
                }
                BeltKind::Splitter => {
                    let di = dir_index(b.direction);
                    let key = format!("ent:{name}:{di}");
                    let l =
                        sprites.get(assets, data, &key, || factorio_data::sprite::entity_sprite_dir(&d, &name, di))?;
                    Some((key, l))
                }
            }
        }
        (_, EntityData::Pipe { .. }) if proto.kind == "pipe" => {
            let t = e.position.tile();
            let mut mask = 0;
            for (bit, dir) in Direction::CARDINALS.iter().enumerate() {
                if let Some(other) = sim.0.entity_at_tile(t.step(*dir))
                    && matches!(sim.0.entity(other).map(|o| &o.state), Some(EntityState::Fluid(_)))
                {
                    mask |= 1 << bit;
                }
            }
            let key_name = match mask {
                0b0000 => "straight_vertical_single",
                0b0001 => "ending_up",
                0b0010 => "ending_right",
                0b0100 => "ending_down",
                0b1000 => "ending_left",
                0b0101 => "straight_vertical",
                0b1010 => "straight_horizontal",
                0b0011 => "corner_up_right",
                0b1001 => "corner_up_left",
                0b0110 => "corner_down_right",
                0b1100 => "corner_down_left",
                0b0111 => "t_right",
                0b1101 => "t_left",
                0b1110 => "t_down",
                0b1011 => "t_up",
                _ => "cross",
            };
            let key = format!("pipe:{name}:{key_name}");
            let l = sprites.get(assets, data, &key, || factorio_data::sprite::pipe_picture(&d, &name, key_name))?;
            Some((key, l))
        }
        _ => {
            let di = dir_index(e.direction);
            let key = format!("ent:{name}:{di}");
            let l = sprites.get(assets, data, &key, || {
                factorio_data::sprite::entity_sprite_dir(&d, &name, di)
                    .or_else(|| factorio_data::sprite::entity_sprite(&d, &name))
            })?;
            Some((key, l))
        }
    }
}

fn sync_entities(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    mut sprites: ResMut<Sprites>,
    mut mirror: ResMut<Mirror>,
    mut q: Query<(&mut Sprite, &mut Transform)>,
) {
    let live: HashSet<EntityId> = sim.0.entities().map(|(id, _)| id).collect();
    mirror.0.retain(|id, m| {
        let keep = live.contains(id);
        if !keep {
            for l in &m.layers {
                commands.entity(*l).despawn();
            }
            if let Some(h) = m.hand {
                commands.entity(h).despawn();
            }
        }
        keep
    });

    let db = sim.0.prototypes();
    for (id, e) in sim.0.entities() {
        let proto = db.entity(e.proto);
        let pos = map_to_world(e.position);
        let look = entity_look(&sim, &data, &mut sprites, &assets, id);
        let layer = if matches!(e.state, EntityState::Belt) { -10.0 } else { 0.0 };
        let entry = mirror.0.entry(id).or_insert_with(|| {
            let hand = if let EntityData::Inserter { .. } = proto.data {
                let d = data.0.clone();
                let name = proto.name.clone();
                sprites
                    .get(&assets, &data, &format!("hand:{name}"), || {
                        factorio_data::sprite::inserter_hand(&d, &name).map(|h| h.0)
                    })
                    .map(|l| commands.spawn((l.sprite(), Transform::from_xyz(pos.x, pos.y, 5.0))).id())
            } else {
                None
            };
            Mirrored { layers: Vec::new(), hand, key: String::new() }
        });
        let m = entry;
        match &look {
            Some((key, layers)) if *key != m.key => {
                while m.layers.len() < layers.len() {
                    m.layers.push(commands.spawn((Sprite::default(), Transform::default())).id());
                }
                while m.layers.len() > layers.len() {
                    commands.entity(m.layers.pop().unwrap()).despawn();
                }
                for (i, ((l, kind), ent)) in layers.iter().zip(&m.layers).enumerate() {
                    let at = pos + l.shift();
                    let (z, color) = match kind {
                        LayerKind::Shadow => (-3.0 + i as f32 * 1e-6, Color::srgba(0.0, 0.0, 0.0, 0.55)),
                        LayerKind::Normal => (depth(pos.y, layer) + i as f32 * 1e-6, Color::WHITE),
                    };
                    match q.get_mut(*ent) {
                        Ok((mut sprite, mut tf)) => {
                            l.apply(&mut sprite);
                            sprite.color = color;
                            tf.translation = Vec3::new(at.x, at.y, z);
                        }
                        Err(_) => {
                            // Freshly spawned this frame: insert the full components.
                            let mut sprite = l.sprite();
                            sprite.color = color;
                            commands.entity(*ent).insert((sprite, Transform::from_xyz(at.x, at.y, z)));
                        }
                    }
                }
                m.key = key.clone();
            }
            None if m.layers.is_empty() => {
                let e = commands
                    .spawn((
                        Sprite::from_color(Color::srgba(0.8, 0.2, 0.8, 0.8), Vec2::splat(TILE * 0.8)),
                        Transform::from_xyz(pos.x, pos.y, depth(pos.y, 0.0)),
                    ))
                    .id();
                m.layers.push(e);
            }
            _ => {}
        }
        {
            {
                // Inserter hand position from the arm's angle and length.
                if let (Some(h), EntityState::Inserter(ins), EntityData::Inserter { pickup_position, .. }) =
                    (m.hand, &e.state, &proto.data)
                    && let Ok((_, mut tf)) = q.get_mut(h)
                {
                    let [px, py] = e.direction.rotate_vec([
                        (pickup_position[0].to_f64_lossy() * 256.0) as i32,
                        (pickup_position[1].to_f64_lossy() * 256.0) as i32,
                    ]);
                    let base = (-(py as f32)).atan2(px as f32);
                    let angle = base - ins.rotation.to_f64_lossy() as f32 * TAU;
                    let len = ins.extension.to_f64_lossy() as f32 * TILE;
                    // The arm sprite spans from the inserter's centre to the hand.
                    let mid = pos + Vec2::new(angle.cos(), angle.sin()) * (len / 2.0);
                    tf.translation = Vec3::new(mid.x, mid.y, 5.0);
                    tf.scale = Vec3::new(1.0, len / (164.0 * 0.25), 1.0);
                    tf.rotation = Quat::from_rotation_z(angle - std::f32::consts::FRAC_PI_2);
                }
            }
        }
    }
}

/// Items on belts and on the ground, drawn from a reusable pool of sprites.
fn draw_items(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    mut sprites: ResMut<Sprites>,
    mut pools: ResMut<Pools>,
    mut q: Query<(&mut Sprite, &mut Transform, &mut Visibility)>,
) {
    let db = sim.0.prototypes();
    let mut wanted: Vec<(String, Vec2, f32, f32)> = Vec::new();
    for b in sim.0.belts.belts.values() {
        let centre = map_to_world(b.position);
        for (lane, l) in b.lanes.iter().enumerate() {
            for item in &l.items {
                let [ox, oy] = b.item_offset(lane, item.pos);
                let p = centre + Vec2::new(ox as f32, -(oy as f32)) * (TILE / 256.0);
                wanted.push((db.item(item.item).name.clone(), p, -5.0, 0.45));
            }
        }
    }
    for (p, stack) in sim.0.ground_items() {
        wanted.push((db.item(stack.item).name.clone(), map_to_world(p), -6.0, 0.45));
    }
    while pools.items.len() < wanted.len() {
        let e = commands.spawn((Sprite::default(), Transform::default(), Visibility::Hidden)).id();
        pools.items.push(e);
    }
    for (i, e) in pools.items.iter().enumerate() {
        let Ok((mut sprite, mut tf, mut vis)) = q.get_mut(*e) else { continue };
        match wanted.get(i) {
            Some((name, p, z, size)) => {
                if let Some(icon) = sprites.item_icon(&assets, &data, name) {
                    icon.apply(&mut sprite);
                    sprite.custom_size = Some(Vec2::splat(TILE * size));
                }
                tf.translation = Vec3::new(p.x, p.y, *z);
                *vis = Visibility::Visible;
            }
            None => *vis = Visibility::Hidden,
        }
    }
}

fn spawn_character(mut commands: Commands) {
    commands.spawn((CharacterSprite, Sprite::default(), Transform::default()));
}

#[derive(Default)]
struct Facing(u32);

fn draw_character(
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    mut sprites: ResMut<Sprites>,
    mut q: Query<(&mut Sprite, &mut Transform), With<CharacterSprite>>,
    mut facing: Local<Facing>,
) {
    let Some(c) = sim.0.player(LOCAL_PLAYER).and_then(|p| p.character.as_ref()) else { return };
    let Ok((mut sprite, mut tf)) = q.single_mut() else { return };
    let running = c.walking.is_some();
    if let Some(d) = c.walking {
        facing.0 = (d.0 / 2) as u32;
    }
    let tick = sim.0.tick();
    let frame = if running { (tick / 2) as u32 } else { (tick / 6) as u32 };
    let name = sim.0.prototypes().entity(c.proto).name.clone();
    let d = data.0.clone();
    let dir = facing.0;
    let key = format!("char:{name}:{dir}:{running}:{}", frame % 22);
    let pos = map_to_world(c.position());
    if let Some(l) =
        sprites.get(&assets, &data, &key, || factorio_data::sprite::character_sprite(&d, &name, dir, running, frame))
    {
        l.apply(&mut sprite);
        let at = pos + l.shift();
        tf.translation = Vec3::new(at.x, at.y, depth(pos.y, 0.0) + 0.00005);
    } else {
        *sprite = Sprite::from_color(Color::srgb(1.0, 0.6, 0.1), Vec2::splat(TILE * 0.5));
        tf.translation = Vec3::new(pos.x, pos.y, 1.0);
    }
}

fn draw_ghost(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    cursor: Res<Cursor>,
    mouse: Res<crate::controls::MouseWorld>,
    ui: Res<crate::controls::UiState>,
    mut sprites: ResMut<Sprites>,
    mut ghosts: Query<(Entity, &mut Sprite, &mut Transform), With<Ghost>>,
) {
    let db = sim.0.prototypes();
    let held = sim.0.player(LOCAL_PLAYER).and_then(|p| p.character.as_ref()).and_then(|c| c.cursor);
    let target = held.and_then(|s| db.item(s.item).place_result).zip(mouse.0).filter(|_| !ui.pointer_over_ui);
    let Some((entity, at)) = target else {
        for (e, _, _) in ghosts.iter() {
            commands.entity(e).despawn();
        }
        return;
    };
    let proto = db.entity(entity);
    let dir = if proto.rotatable { cursor.direction } else { Direction::NORTH };
    let snapped = proto.snap_position(at, dir);
    let ok = sim.0.can_place(entity, snapped, dir).is_ok();
    let name = proto.name.clone();
    let d = data.0.clone();
    let di = dir_index(dir);
    // Every non-shadow layer, first animation frame, like the game's placement preview.
    let layers: Vec<Loaded> = if matches!(proto.data, EntityData::TransportBelt { .. }) {
        let index = format!("{}_index", dir_name(dir));
        sprites
            .get(&assets, &data, &format!("belt:{name}:{index}:0"), || {
                factorio_data::sprite::belt_sheet(&d, &name).map(|s| s.frame(&index, 0))
            })
            .into_iter()
            .collect()
    } else {
        let l: Vec<Loaded> = factorio_data::sprite::entity_layers(&d, &name, di, 0, false)
            .into_iter()
            .filter(|(_, k)| *k == LayerKind::Normal)
            .map(|(sprite, _)| Loaded { image: assets.load(crate::sprites::asset_path(&data, &sprite.path)), sprite })
            .collect();
        if l.is_empty() {
            sprites
                .get(&assets, &data, &format!("ent:{name}:{di}"), || {
                    factorio_data::sprite::entity_sprite_dir(&d, &name, di)
                })
                .into_iter()
                .collect()
        } else {
            l
        }
    };
    let pos = map_to_world(snapped);
    let color = if ok { Color::srgba(0.6, 1.0, 0.6, 0.6) } else { Color::srgba(1.0, 0.3, 0.3, 0.6) };
    let mut existing: Vec<(Entity, Mut<Sprite>, Mut<Transform>)> = ghosts.iter_mut().collect();
    while existing.len() > layers.len() {
        let (e, _, _) = existing.pop().unwrap();
        commands.entity(e).despawn();
    }
    for (i, l) in layers.iter().enumerate() {
        let at = pos + l.shift();
        let z = 50.0 + i as f32 * 0.001;
        match existing.get_mut(i) {
            Some((_, s, tf)) => {
                l.apply(s);
                s.color = color;
                tf.translation = Vec3::new(at.x, at.y, z);
            }
            None => {
                let mut sprite = l.sprite();
                sprite.color = color;
                commands.spawn((Ghost, sprite, Transform::from_xyz(at.x, at.y, z)));
            }
        }
    }
}

fn follow_camera(sim: Res<Sim>, mut cam: Query<&mut Transform, With<Camera2d>>) {
    let Some(c) = sim.0.player(LOCAL_PLAYER).and_then(|p| p.character.as_ref()) else { return };
    let Ok(mut tf) = cam.single_mut() else { return };
    let p = map_to_world(c.position());
    tf.translation.x = p.x;
    tf.translation.y = p.y;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_rows_name_the_entry_side() {
        // Items going east that turn north (fed from the west side): a left turn.
        assert_eq!(belt_row_name(Direction::NORTH, BeltShape::CurveLeft), "west_to_north_index");
        // Items going west that turn north (fed from the east side): a right turn.
        assert_eq!(belt_row_name(Direction::NORTH, BeltShape::CurveRight), "east_to_north_index");
        assert_eq!(belt_row_name(Direction::EAST, BeltShape::CurveLeft), "north_to_east_index");
        assert_eq!(belt_row_name(Direction::SOUTH, BeltShape::Straight), "south_index");
    }
}
