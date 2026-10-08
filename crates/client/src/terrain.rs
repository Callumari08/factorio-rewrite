//! Draws the ground: one texture per chunk, composed on the CPU from the game's tile
//! textures, plus resource deposits as sprites.
//!
//! Tile edges use the game's transition masks: where a tile borders one of a higher
//! `layer`, the higher tile is drawn into it through mask pieces chosen from which sides
//! and corners it touches (side, inner corner, outer corner, U and O pieces).

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
    /// Downscaled transition masks per tile (`None` for tiles without any).
    masks: HashMap<TileId, Option<Masks>>,
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

/// A tile's transition mask pieces at PX×PX: per variant, the four rotations (N, E, S, W).
struct Masks {
    inner: Vec<[Vec<u8>; 4]>,
    outer: Vec<[Vec<u8>; 4]>,
    side: Vec<[Vec<u8>; 4]>,
    u: Vec<[Vec<u8>; 4]>,
    o: Vec<[Vec<u8>; 4]>,
}

impl Terrain {
    fn masks(&mut self, data: &Data, sim: &Sim, tile: TileId) -> Option<&Masks> {
        if let std::collections::hash_map::Entry::Vacant(slot) = self.masks.entry(tile) {
            let name = sim.0.prototypes().tile(tile).name.clone();
            let loaded = factorio_data::sprite::tile_transition(&data.0, &name).and_then(|t| {
                let img = image::open(&t.sheet).ok()?.to_luma8();
                let cut = |p: factorio_data::sprite::MaskPieces| -> Vec<[Vec<u8>; 4]> {
                    (0..p.count)
                        .filter_map(|v| {
                            let rot = |r: u32| -> Option<Vec<u8>> {
                                let (x, y) = (p.x + v * t.size, t.y + r * t.size);
                                (x + t.size <= img.width() && y + t.size <= img.height()).then(|| {
                                    let crop = image::imageops::crop_imm(&img, x, y, t.size, t.size).to_image();
                                    image::imageops::resize(&crop, PX, PX, image::imageops::FilterType::Triangle)
                                        .into_raw()
                                })
                            };
                            Some([rot(0)?, rot(1)?, rot(2)?, rot(3)?])
                        })
                        .collect()
                };
                Some(Masks {
                    inner: cut(t.inner_corner),
                    outer: cut(t.outer_corner),
                    side: cut(t.side),
                    u: cut(t.u_transition),
                    o: cut(t.o_transition),
                })
            });
            slot.insert(loaded);
        }
        self.masks[&tile].as_ref()
    }
}

/// The mask through which a higher tile covers a cell, from where it touches the cell:
/// `edges` N, E, S, W and `corners` NE, SE, SW, NW. `None` when it does not touch it.
fn transition_mask(m: &Masks, edges: [bool; 4], corners: [bool; 4], pick: u32) -> Option<Vec<u8>> {
    let n = (PX * PX) as usize;
    let mut out = vec![0u8; n];
    let mut any = false;
    let mut add = |set: &Vec<[Vec<u8>; 4]>, rot: usize| {
        if set.is_empty() {
            return;
        }
        let piece = &set[pick as usize % set.len()][rot];
        for (o, v) in out.iter_mut().zip(piece) {
            *o = (*o).max(*v);
        }
        any = true;
    };
    let count = edges.iter().filter(|e| **e).count();
    match count {
        4 => add(&m.o, 0),
        // The U piece's rotation is the side it is open on, turned by two.
        3 => add(&m.u, (edges.iter().position(|e| !*e).unwrap() + 2) % 4),
        2 if edges[0] == edges[2] => {
            // Opposite sides.
            for (r, e) in edges.iter().enumerate() {
                if *e {
                    add(&m.side, r);
                }
            }
        }
        2 => {
            // Two adjacent sides: the inner corner between them (rotation of the first).
            let r = (0..4).find(|r| edges[*r] && edges[(r + 1) % 4]).unwrap();
            add(&m.inner, r);
        }
        1 => add(&m.side, edges.iter().position(|e| *e).unwrap()),
        _ => {}
    }
    // Diagonal neighbours count where neither adjacent side does.
    for (c, present) in corners.iter().enumerate() {
        if *present && !edges[c] && !edges[(c + 1) % 4] {
            add(&m.outer, c);
        }
    }
    any.then_some(out)
}

fn build_chunks(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    mut terrain: ResMut<Terrain>,
    mut images: ResMut<Assets<Image>>,
) {
    // A chunk is drawn once its neighbours exist, so its edges can blend into them.
    let surface = &sim.0.surface;
    let todo: Vec<ChunkPosition> = surface
        .chunks()
        .map(|(p, _)| p)
        .filter(|p| !terrain.chunks.contains_key(p))
        .filter(|p| {
            (-1..=1).all(|dy| (-1..=1).all(|dx| surface.is_generated(ChunkPosition { x: p.x + dx, y: p.y + dy })))
        })
        .take(6)
        .collect();
    let db = sim.0.prototypes();
    for c in todo {
        let size = CHUNK_SIZE as u32 * PX;
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        let first = c.first_tile();
        for ty in 0..CHUNK_SIZE {
            for tx in 0..CHUNK_SIZE {
                let t = TilePosition::new(first.x + tx, first.y + ty);
                let Some(tile) = sim.0.surface.tile(t) else { continue };
                let pick = hash(7, 1, t.x, t.y) as usize;
                let variants = terrain.tile_textures(&data, &sim, tile);
                let mut cell = variants[pick % variants.len()].clone();
                // Neighbours N, E, S, W, then NE, SE, SW, NW.
                let around = [(0, -1), (1, 0), (0, 1), (-1, 0), (1, -1), (1, 1), (-1, 1), (-1, -1)]
                    .map(|(dx, dy)| sim.0.surface.tile(TilePosition::new(t.x + dx, t.y + dy)));
                let layer = db.tile(tile).layer;
                let mut higher: Vec<TileId> =
                    around.iter().flatten().copied().filter(|n| db.tile(*n).layer > layer).collect();
                higher.sort_by_key(|n| (db.tile(*n).layer, *n));
                higher.dedup();
                for over in higher {
                    let edges = [0, 1, 2, 3].map(|i| around[i] == Some(over));
                    let corners = [4, 5, 6, 7].map(|i| around[i] == Some(over));
                    let piece = hash(7, 2 + over.0 as u32, t.x, t.y);
                    let Some(mask) =
                        terrain.masks(&data, &sim, over).and_then(|m| transition_mask(m, edges, corners, piece))
                    else {
                        continue;
                    };
                    let top = terrain.tile_textures(&data, &sim, over);
                    let top = &top[pick % top.len()];
                    for (i, m) in mask.iter().enumerate() {
                        let a = *m as u32;
                        for ch in 0..3 {
                            let (b, o) = (cell[i * 4 + ch] as u32, top[i * 4 + ch] as u32);
                            cell[i * 4 + ch] = ((b * (255 - a) + o * a) / 255) as u8;
                        }
                    }
                }
                for py in 0..PX {
                    let src = (py * PX * 4) as usize;
                    let dst = (((ty as u32 * PX + py) * size + tx as u32 * PX) * 4) as usize;
                    pixels[dst..dst + (PX * 4) as usize].copy_from_slice(&cell[src..src + (PX * 4) as usize]);
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
