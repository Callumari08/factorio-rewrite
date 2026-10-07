//! Draws the ground: one texture per chunk, composed on the CPU from the game's tile
//! textures, plus resource deposits as sprites.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use factorio_sim::map::{CHUNK_SIZE, ChunkPosition, MapPosition, TilePosition};
use factorio_sim::noise::hash;
use factorio_sim::proto::{EntityData, EntityProtoId, TileId};

use crate::sprites::Sprites;
use crate::{Data, Sim, TILE};

/// Pixels per tile in the composed chunk textures.
const PX: u32 = 16;

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Terrain>().add_systems(Update, (build_chunks, update_resources));
    }
}

#[derive(Default)]
struct ChunkView {
    scanned: bool,
    resources: HashMap<TilePosition, (Entity, EntityProtoId, u32)>,
}

#[derive(Resource, Default)]
struct Terrain {
    chunks: HashMap<ChunkPosition, ChunkView>,
    /// Downscaled tile textures (PX×PX RGBA) per variant.
    textures: HashMap<TileId, Vec<Vec<u8>>>,
    files: HashMap<std::path::PathBuf, Option<image::RgbaImage>>,
    frame: u32,
}

impl Terrain {
    fn tile_textures(&mut self, data: &Data, sim: &Sim, tile: TileId) -> &Vec<Vec<u8>> {
        if !self.textures.contains_key(&tile) {
            let proto = sim.0.prototypes().tile(tile);
            let mut out = Vec::new();
            for v in factorio_data::sprite::tile_variants(&data.0, &proto.name) {
                let img = self
                    .files
                    .entry(v.path.clone())
                    .or_insert_with(|| image::open(&v.path).ok().map(|i| i.to_rgba8()))
                    .as_ref();
                if let Some(img) = img
                    && v.x + v.width <= img.width()
                    && v.y + v.height <= img.height()
                {
                    let crop = image::imageops::crop_imm(img, v.x, v.y, v.width, v.height).to_image();
                    let small = image::imageops::resize(&crop, PX, PX, image::imageops::FilterType::Triangle);
                    out.push(small.into_raw());
                }
            }
            if out.is_empty() {
                let [r, g, b] = proto.map_color;
                out.push([r, g, b, 255].repeat((PX * PX) as usize));
            }
            self.textures.insert(tile, out);
        }
        &self.textures[&tile]
    }
}

fn build_chunks(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    mut terrain: ResMut<Terrain>,
    mut images: ResMut<Assets<Image>>,
) {
    let todo: Vec<ChunkPosition> =
        sim.0.surface.chunks().map(|(p, _)| p).filter(|p| !terrain.chunks.contains_key(p)).take(6).collect();
    for c in todo {
        let size = CHUNK_SIZE as u32 * PX;
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        let first = c.first_tile();
        for ty in 0..CHUNK_SIZE {
            for tx in 0..CHUNK_SIZE {
                let t = TilePosition::new(first.x + tx, first.y + ty);
                let Some(tile) = sim.0.surface.tile(t) else { continue };
                let variants = terrain.tile_textures(&data, &sim, tile);
                let v = &variants[(hash(7, 1, t.x, t.y) as usize) % variants.len()];
                for py in 0..PX {
                    let src = (py * PX * 4) as usize;
                    let dst = (((ty as u32 * PX + py) * size + tx as u32 * PX) * 4) as usize;
                    pixels[dst..dst + (PX * 4) as usize].copy_from_slice(&v[src..src + (PX * 4) as usize]);
                }
            }
        }
        let image = Image::new(
            Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            TextureDimension::D2,
            pixels,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        let handle = images.add(image);
        let world = crate::map_to_world(MapPosition::from_tiles(first.x, first.y))
            + Vec2::new(1.0, -1.0) * (CHUNK_SIZE as f32 * TILE / 2.0);
        commands.spawn((
            Sprite { image: handle, custom_size: Some(Vec2::splat(CHUNK_SIZE as f32 * TILE)), ..default() },
            Transform::from_xyz(world.x, world.y, -100.0),
        ));
        terrain.chunks.insert(c, ChunkView::default());
    }
}

fn stage_for(stage_counts: &[u32], amount: u32) -> u32 {
    stage_counts.iter().position(|c| amount >= *c).unwrap_or(stage_counts.len().saturating_sub(1)) as u32
}

fn update_resources(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    mut sprites: ResMut<Sprites>,
    mut terrain: ResMut<Terrain>,
) {
    terrain.frame += 1;
    let refresh_all = terrain.frame.is_multiple_of(30);
    let db = sim.0.prototypes();
    let keys: Vec<ChunkPosition> = terrain.chunks.keys().copied().collect();
    for c in keys {
        let fresh = !terrain.chunks[&c].scanned;
        if !refresh_all && !fresh {
            continue;
        }
        terrain.chunks.get_mut(&c).unwrap().scanned = true;
        let first = c.first_tile();
        for ty in 0..CHUNK_SIZE {
            for tx in 0..CHUNK_SIZE {
                let t = TilePosition::new(first.x + tx, first.y + ty);
                let want = sim.0.surface.resource(t).map(|r| {
                    let stages = match &db.entity(r.proto).data {
                        EntityData::Resource { stage_counts, .. } => stage_counts.clone(),
                        _ => Vec::new(),
                    };
                    (r.proto, stage_for(&stages, r.amount))
                });
                let view = terrain.chunks.get_mut(&c).unwrap();
                let have = view.resources.get(&t).map(|(_, p, s)| (*p, *s));
                if have == want {
                    continue;
                }
                if let Some((e, _, _)) = view.resources.remove(&t) {
                    commands.entity(e).despawn();
                }
                if let Some((proto, stage)) = want {
                    let name = db.entity(proto).name.clone();
                    let variation = hash(3, 2, t.x, t.y) % 8;
                    let d = data.0.clone();
                    let key = format!("resource:{name}:{stage}:{variation}");
                    if let Some(s) = sprites.get(&assets, &data, &key, || {
                        factorio_data::sprite::resource_sprite(&d, &name, stage, variation)
                    }) {
                        let p = crate::map_to_world(MapPosition::tile_center(t)) + s.shift();
                        let e = commands.spawn((s.sprite(), Transform::from_xyz(p.x, p.y, -50.0))).id();
                        terrain.chunks.get_mut(&c).unwrap().resources.insert(t, (e, proto, stage));
                    }
                }
            }
        }
    }
}
