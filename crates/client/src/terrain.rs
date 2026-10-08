//! Draws the ground: textures per chunk, composed on the CPU from the game's tile
//! textures and decoratives, plus resource deposits as sprites.
//!
//! Chunks in view get a texture at the detail the zoom needs: the game's full 64 px art at
//! normal zoom and closer (as on its high graphics quality), 32 px when a little zoomed
//! out, and a small 8 px one that every visible chunk keeps (shown when far out or while
//! a detailed one is being made). Textures are composed a tile row at a time within a
//! per-frame budget, and detailed ones out of view are dropped again.
//!
//! Tiles use their 1x1, 2x2 and 4x4 pictures (the bigger ones wherever an aligned block is
//! all the same tile, as the game does). Tile edges use the game's transition masks: where
//! a tile borders one of a higher `layer`, the higher tile is drawn into it through mask
//! pieces chosen from which sides and corners it touches.

use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use factorio_sim::map::{CHUNK_SIZE, ChunkPosition, MapPosition, TilePosition};
use factorio_sim::noise::hash;
use factorio_sim::proto::{EntityData, EntityProtoId, TileId};

use crate::sprites::Sprites;
use crate::{Data, Sim, TILE};

/// Pixels per tile of the chunk textures: the game's full art (64), half (32) and small (8).
const HI: u32 = 64;
const MID: u32 = 32;
const LO: u32 = 8;
/// Above this camera scale (zoomed out), only small textures are drawn.
const MID_MAX_SCALE: f32 = 2.5;
/// Time per frame spent composing chunk textures.
const BUDGET_MS: f64 = 8.0;

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Terrain>()
            .init_resource::<GroundReady>()
            .add_systems(Update, (build_chunks, update_resources).chain());
    }
}

/// Chunks whose ground is drawn; entities on other chunks are not drawn yet.
#[derive(Resource, Default)]
pub struct GroundReady(pub HashSet<ChunkPosition>);

#[derive(Default)]
struct ChunkView {
    scanned: bool,
    resources: HashMap<TilePosition, (Entity, EntityProtoId, u32)>,
    /// Textures by pixels per tile.
    levels: HashMap<u32, (Entity, Handle<Image>)>,
}

/// Per pixel density: tile pictures by block size, masks and decoratives.
#[derive(Default)]
struct Level {
    /// (tile, block size) → variants, each (size*px)² RGBA.
    blocks: HashMap<(TileId, u32), Vec<Vec<u8>>>,
    masks: HashMap<TileId, Option<Masks>>,
    decoratives: HashMap<(u16, u8), Option<(image::RgbaImage, i32, i32)>>,
    shores: HashMap<TileId, Vec<ShoreSet>>,
}

#[derive(Resource, Default)]
struct Terrain {
    chunks: HashMap<ChunkPosition, ChunkView>,
    levels: HashMap<u32, Level>,
    /// Decoratives per chunk, kept so neighbouring chunks can draw the parts that reach
    /// over their edge.
    placed: HashMap<ChunkPosition, Vec<factorio_sim::mapgen::PlacedDecorative>>,
    files: HashMap<std::path::PathBuf, Option<image::RgbaImage>>,
    frame: u32,
    /// The texture being composed.
    job: Option<Job>,
}

fn load_file<'a>(
    files: &'a mut HashMap<std::path::PathBuf, Option<image::RgbaImage>>,
    path: &std::path::Path,
) -> Option<&'a image::RgbaImage> {
    files.entry(path.to_owned()).or_insert_with(|| image::open(path).ok().map(|i| i.to_rgba8())).as_ref()
}

impl Terrain {
    /// The variants of a tile's `size`×`size` block picture at `px` per tile; falls back to
    /// single tiles (and to the map colour when there is no picture).
    fn blocks(&mut self, data: &Data, sim: &Sim, tile: TileId, size: u32, px: u32) -> &Vec<Vec<u8>> {
        let key = (tile, size);
        if !self.levels.entry(px).or_default().blocks.contains_key(&key) {
            let proto = sim.0.prototypes().tile(tile);
            let mut out = Vec::new();
            for (s, sprites) in factorio_data::sprite::tile_variant_sets(&data.0, &proto.name) {
                if s != size {
                    continue;
                }
                for v in sprites {
                    let Some(img) = load_file(&mut self.files, &v.path) else { continue };
                    if v.x + v.width > img.width() || v.y + v.height > img.height() {
                        continue;
                    }
                    let crop = image::imageops::crop_imm(img, v.x, v.y, v.width, v.height).to_image();
                    let n = px * size;
                    out.push(image::imageops::resize(&crop, n, n, image::imageops::FilterType::CatmullRom).into_raw());
                }
            }
            if out.is_empty() && size == 1 {
                let [r, g, b] = proto.map_color;
                out.push([r, g, b, 255].repeat((px * px) as usize));
            }
            self.levels.get_mut(&px).unwrap().blocks.insert(key, out);
        }
        &self.levels[&px].blocks[&key]
    }

    /// One tile's picture: from a 4x4 or 2x2 block picture when its aligned block is all
    /// this tile, else a single-tile variant.
    fn tile_pixels(&mut self, data: &Data, sim: &Sim, tile: TileId, t: TilePosition, px: u32) -> Vec<u8> {
        let surface = &sim.0.surface;
        for size in [4i32, 2] {
            let (bx, by) = (t.x.div_euclid(size) * size, t.y.div_euclid(size) * size);
            let whole =
                (0..size).all(|dy| (0..size).all(|dx| surface.tile(TilePosition::new(bx + dx, by + dy)) == Some(tile)));
            if !whole {
                continue;
            }
            let pick = hash(7, 10 + size as u32, bx, by) as usize;
            let set = self.blocks(data, sim, tile, size as u32, px);
            if set.is_empty() {
                continue;
            }
            let block = &set[pick % set.len()];
            let (ox, oy) = ((t.x - bx) as u32 * px, (t.y - by) as u32 * px);
            let stride = (size as u32 * px * 4) as usize;
            let mut out = Vec::with_capacity((px * px * 4) as usize);
            for y in 0..px {
                let start = (oy + y) as usize * stride + (ox * 4) as usize;
                out.extend_from_slice(&block[start..start + (px * 4) as usize]);
            }
            return out;
        }
        let set = self.blocks(data, sim, tile, 1, px);
        set[hash(7, 1, t.x, t.y) as usize % set.len()].clone()
    }
}

/// A tile's transition mask pieces at px×px: per variant, the four rotations (N, E, S, W).
struct Masks {
    inner: Vec<[Vec<u8>; 4]>,
    outer: Vec<[Vec<u8>; 4]>,
    side: Vec<[Vec<u8>; 4]>,
    u: Vec<[Vec<u8>; 4]>,
    o: Vec<[Vec<u8>; 4]>,
}

impl Terrain {
    fn masks(&mut self, data: &Data, sim: &Sim, tile: TileId, px: u32) -> Option<&Masks> {
        let level = self.levels.entry(px).or_default();
        if let std::collections::hash_map::Entry::Vacant(slot) = level.masks.entry(tile) {
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
                                    image::imageops::resize(&crop, px, px, image::imageops::FilterType::CatmullRom)
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
        self.levels[&px].masks[&tile].as_ref()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Piece {
    Inner,
    Outer,
    Side,
    U,
    O,
}

/// The transition pieces (and rotations) for a cell that a higher tile touches on sides
/// `edges` (N, E, S, W) and diagonals `corners` (NE, SE, SW, NW).
fn pick_pieces(edges: [bool; 4], corners: [bool; 4]) -> Vec<(Piece, usize)> {
    let mut out = Vec::new();
    match edges.iter().filter(|e| **e).count() {
        4 => out.push((Piece::O, 0)),
        // The U piece's rotation is the side it is open on, turned by two.
        3 => out.push((Piece::U, (edges.iter().position(|e| !*e).unwrap() + 2) % 4)),
        // Opposite sides.
        2 if edges[0] == edges[2] => out.extend((0..4).filter(|r| edges[*r]).map(|r| (Piece::Side, r))),
        // Two adjacent sides: the inner corner between them (rotation of the first).
        2 => out.push((Piece::Inner, (0..4).find(|r| edges[*r] && edges[(r + 1) % 4]).unwrap())),
        1 => out.push((Piece::Side, edges.iter().position(|e| *e).unwrap())),
        _ => {}
    }
    // Diagonal neighbours count where neither adjacent side does.
    for (c, present) in corners.iter().enumerate() {
        if *present && !edges[c] && !edges[(c + 1) % 4] {
            out.push((Piece::Outer, c));
        }
    }
    out
}

/// The mask through which a higher tile covers a cell. `None` when it does not touch it.
fn transition_mask(m: &Masks, edges: [bool; 4], corners: [bool; 4], pick: u32, px: u32) -> Option<Vec<u8>> {
    let pieces = pick_pieces(edges, corners);
    if pieces.is_empty() {
        return None;
    }
    let mut out = vec![0u8; (px * px) as usize];
    for (kind, rot) in pieces {
        let set = match kind {
            Piece::Inner => &m.inner,
            Piece::Outer => &m.outer,
            Piece::Side => &m.side,
            Piece::U => &m.u,
            Piece::O => &m.o,
        };
        if set.is_empty() {
            continue;
        }
        for (o, v) in out.iter_mut().zip(&set[pick as usize % set.len()][rot]) {
            *o = (*o).max(*v);
        }
    }
    Some(out)
}

/// Soft edges where two water tiles meet (water and deep water have no masks of their own;
/// the game blends them in its water shader): a smooth ramp from each touching side.
fn soft_mask(edges: [bool; 4], corners: [bool; 4], px: u32) -> Option<Vec<u8>> {
    if !edges.iter().chain(&corners).any(|e| *e) {
        return None;
    }
    let mut out = vec![0u8; (px * px) as usize];
    for y in 0..px {
        for x in 0..px {
            let (fx, fy) = ((x as f32 + 0.5) / px as f32, (y as f32 + 0.5) / px as f32);
            // Distance (in tiles) to each touching side or corner.
            let mut d = f32::INFINITY;
            let side = [fy, 1.0 - fx, 1.0 - fy, fx];
            for (i, e) in edges.iter().enumerate() {
                if *e {
                    d = d.min(side[i]);
                }
            }
            let corner = [(1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)];
            for (i, c) in corners.iter().enumerate() {
                if *c {
                    let (cx, cy) = corner[i];
                    d = d.min(((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt());
                }
            }
            let t = (1.0 - d).clamp(0.0, 1.0);
            out[(y * px + x) as usize] = (t * t * (3.0 - 2.0 * t) * 255.0) as u8;
        }
    }
    Some(out)
}

/// A land tile's shore pieces at px per tile: per piece kind, variants × rotations, each
/// mask px×px and overlay/background px×(px*tile_height) RGBA.
struct ShoreSet {
    to_tiles: Vec<TileId>,
    pieces: Vec<(Piece, u32, Vec<[ShorePiece; 4]>)>,
}

#[derive(Clone)]
struct ShorePiece {
    mask: Vec<u8>,
    overlay: Vec<u8>,
    background: Option<Vec<u8>>,
}

impl Terrain {
    fn shores(&mut self, data: &Data, sim: &Sim, tile: TileId, px: u32) -> &Vec<ShoreSet> {
        if !self.levels.entry(px).or_default().shores.contains_key(&tile) {
            let db = sim.0.prototypes();
            let name = db.tile(tile).name.clone();
            let mut sets = Vec::new();
            for sh in factorio_data::sprite::tile_shores(&data.0, &name) {
                let to_tiles: Vec<TileId> = sh.to_tiles.iter().filter_map(|n| db.tile_id(n)).collect();
                let Some(img) = load_file(&mut self.files, &sh.sheet).cloned() else { continue };
                let cut = |x: u32, y: u32, h: u32, out_h: u32| -> Option<Vec<u8>> {
                    (x + sh.size <= img.width() && y + h <= img.height()).then(|| {
                        let c = image::imageops::crop_imm(&img, x, y, sh.size, h).to_image();
                        image::imageops::resize(&c, px, out_h, image::imageops::FilterType::CatmullRom).into_raw()
                    })
                };
                let mut pieces = Vec::new();
                for (kind, p) in [
                    (Piece::Inner, sh.inner_corner),
                    (Piece::Outer, sh.outer_corner),
                    (Piece::Side, sh.side),
                    (Piece::U, sh.u_transition),
                    (Piece::O, sh.o_transition),
                ] {
                    let rotations = if kind == Piece::O { 1 } else { 4 };
                    let h = p.tile_height.max(1);
                    let variants: Vec<[ShorePiece; 4]> = (0..p.count)
                        .filter_map(|v| {
                            let rot = |r: u32| -> Option<ShorePiece> {
                                let x = v * sh.size;
                                Some(ShorePiece {
                                    mask: {
                                        let m = cut(sh.mask_x + x, p.y + r * sh.size, sh.size, px)?;
                                        // The mask's own alpha or brightness selects the land.
                                        m.chunks(4).map(|c| ((c[0] as u32 * c[3] as u32) / 255) as u8).collect()
                                    },
                                    overlay: cut(sh.overlay_x + x, p.y + r * sh.size * h, sh.size * h, px * h)?,
                                    background: sh
                                        .background_x
                                        .and_then(|bx| cut(bx + x, p.y + r * sh.size * h, sh.size * h, px * h)),
                                })
                            };
                            let r0 = rot(0)?;
                            if rotations == 1 {
                                return Some([r0.clone(), r0.clone(), r0.clone(), r0]);
                            }
                            Some([r0, rot(1)?, rot(2)?, rot(3)?])
                        })
                        .collect();
                    pieces.push((kind, h, variants));
                }
                sets.push(ShoreSet { to_tiles, pieces });
            }
            self.levels.get_mut(&px).unwrap().shores.insert(tile, sets);
        }
        &self.levels[&px].shores[&tile]
    }
}

/// Composes a chunk's ground at `px` pixels per tile.
/// A chunk texture being composed a row at a time: tile rows, then shore rows (from the
/// row above the chunk), then decoratives.
struct Job {
    c: ChunkPosition,
    px: u32,
    pixels: Vec<u8>,
    step: i32,
}

const SHORE_STEP: i32 = CHUNK_SIZE;
const DECORATIVE_STEP: i32 = 2 * CHUNK_SIZE + 1;

impl Job {
    fn new(c: ChunkPosition, px: u32) -> Self {
        let size = CHUNK_SIZE as u32 * px;
        Job { c, px, pixels: vec![0u8; (size * size * 4) as usize], step: 0 }
    }
}

/// Does the next step of a job; true when the texture is finished.
fn compose_step(terrain: &mut Terrain, sim: &Sim, data: &Data, job: &mut Job) -> bool {
    let db = sim.0.prototypes();
    let (c, px) = (job.c, job.px);
    let size = CHUNK_SIZE as u32 * px;
    let pixels = &mut job.pixels;
    let first = c.first_tile();
    let step = job.step;
    job.step += 1;
    if step < SHORE_STEP {
        let ty = step;
        for tx in 0..CHUNK_SIZE {
            let t = TilePosition::new(first.x + tx, first.y + ty);
            let Some(tile) = sim.0.surface.tile(t) else { continue };
            let mut cell = terrain.tile_pixels(data, sim, tile, t, px);
            // Neighbours N, E, S, W, then NE, SE, SW, NW.
            let around = [(0, -1), (1, 0), (0, 1), (-1, 0), (1, -1), (1, 1), (-1, 1), (-1, -1)]
                .map(|(dx, dy)| sim.0.surface.tile(TilePosition::new(t.x + dx, t.y + dy)));
            let layer = db.tile(tile).layer;
            let water = db.tile(tile).fluid.is_some();
            // Water tiles share a layer; the later one blends softly over the earlier.
            let mut higher: Vec<TileId> = around
                .iter()
                .flatten()
                .copied()
                .filter(|n| {
                    let o = db.tile(*n);
                    o.layer > layer || (o.layer == layer && water && o.fluid.is_some() && *n > tile)
                })
                .collect();
            higher.sort_by_key(|n| (db.tile(*n).layer, *n));
            higher.dedup();
            for over in higher {
                // Shores are drawn in a second pass with their own pictures.
                if terrain.shores(data, sim, over, px).iter().any(|s| s.to_tiles.contains(&tile)) {
                    continue;
                }
                let edges = [0, 1, 2, 3].map(|i| around[i] == Some(over));
                let corners = [4, 5, 6, 7].map(|i| around[i] == Some(over));
                let piece = hash(7, 2 + over.0 as u32, t.x, t.y);
                let both_water = db.tile(tile).fluid.is_some() && db.tile(over).fluid.is_some();
                let mask = if both_water {
                    soft_mask(edges, corners, px)
                } else {
                    terrain.masks(data, sim, over, px).and_then(|m| transition_mask(m, edges, corners, piece, px))
                };
                let Some(mask) = mask else {
                    continue;
                };
                let top = terrain.tile_pixels(data, sim, over, t, px);
                for (i, m) in mask.iter().enumerate() {
                    let a = *m as u32;
                    for ch in 0..3 {
                        let (b, o) = (cell[i * 4 + ch] as u32, top[i * 4 + ch] as u32);
                        cell[i * 4 + ch] = ((b * (255 - a) + o * a) / 255) as u8;
                    }
                }
            }
            for py in 0..px {
                let src = (py * px * 4) as usize;
                let dst = (((ty as u32 * px + py) * size + tx as u32 * px) * 4) as usize;
                pixels[dst..dst + (px * 4) as usize].copy_from_slice(&cell[src..src + (px * 4) as usize]);
            }
        }
    }
    // Shores: land drawn into neighbouring water through the mask, then the bank on top.
    // Banks reach a tile down, so the row above the chunk is included.
    if (SHORE_STEP..DECORATIVE_STEP).contains(&step) {
        let ty = step - SHORE_STEP - 1;
        for tx in 0..CHUNK_SIZE {
            let t = TilePosition::new(first.x + tx, first.y + ty);
            let Some(tile) = sim.0.surface.tile(t) else { continue };
            let around = [(0, -1), (1, 0), (0, 1), (-1, 0), (1, -1), (1, 1), (-1, 1), (-1, -1)]
                .map(|(dx, dy)| sim.0.surface.tile(TilePosition::new(t.x + dx, t.y + dy)));
            let layer = db.tile(tile).layer;
            let mut lands: Vec<TileId> =
                around.iter().flatten().copied().filter(|n| db.tile(*n).layer > layer).collect();
            lands.sort_by_key(|n| (db.tile(*n).layer, *n));
            lands.dedup();
            for land in lands {
                let edges = [0, 1, 2, 3].map(|i| around[i] == Some(land));
                let corners = [4, 5, 6, 7].map(|i| around[i] == Some(land));
                let pick = hash(7, 40 + land.0 as u32, t.x, t.y) as usize;
                let pieces = pick_pieces(edges, corners);
                let mut chosen: Vec<(ShorePiece, u32)> = Vec::new();
                {
                    let Some(set) = terrain.shores(data, sim, land, px).iter().find(|s| s.to_tiles.contains(&tile))
                    else {
                        continue;
                    };
                    for (kind, rot) in &pieces {
                        if let Some((_, h, variants)) = set.pieces.iter().find(|p| p.0 == *kind)
                            && !variants.is_empty()
                        {
                            chosen.push((variants[pick % variants.len()][*rot].clone(), *h));
                        }
                    }
                }
                let land_px = if ty >= 0 { Some(terrain.tile_pixels(data, sim, land, t, px)) } else { None };
                let blend = |pixels: &mut Vec<u8>, src: &[u8], w: u32, h: u32, x0: i32, y0: i32| {
                    for y in 0..h {
                        let dy = y0 + y as i32;
                        if dy < 0 || dy >= size as i32 {
                            continue;
                        }
                        for x in 0..w {
                            let dx = x0 + x as i32;
                            let s = ((y * w + x) * 4) as usize;
                            let a = src[s + 3] as u32;
                            if a == 0 {
                                continue;
                            }
                            let d = ((dy as u32 * size + dx as u32) * 4) as usize;
                            for ch in 0..3 {
                                pixels[d + ch] =
                                    ((pixels[d + ch] as u32 * (255 - a) + src[s + ch] as u32 * a) / 255) as u8;
                            }
                        }
                    }
                };
                let (x0, y0) = (tx * px as i32, ty * px as i32);
                for (piece, h) in &chosen {
                    if let Some(bg) = &piece.background {
                        blend(pixels, bg, px, px * h, x0, y0);
                    }
                }
                if let Some(land_px) = &land_px {
                    for (piece, _) in &chosen {
                        for y in 0..px {
                            for x in 0..px {
                                let a = piece.mask[(y * px + x) as usize] as u32;
                                if a == 0 {
                                    continue;
                                }
                                let s = ((y * px + x) * 4) as usize;
                                let d = (((y0 as u32 + y) * size + x0 as u32 + x) * 4) as usize;
                                for ch in 0..3 {
                                    pixels[d + ch] =
                                        ((pixels[d + ch] as u32 * (255 - a) + land_px[s + ch] as u32 * a) / 255) as u8;
                                }
                            }
                        }
                    }
                }
                for (piece, h) in &chosen {
                    blend(pixels, &piece.overlay, px, px * h, x0, y0);
                }
            }
        }
    }

    if step < DECORATIVE_STEP {
        return false;
    }
    // Decoratives are painted onto the ground, including the parts of neighbouring
    // chunks' decoratives that reach over the edge.
    let names: Vec<String> = sim
        .0
        .surface
        .settings
        .noise
        .as_ref()
        .map(|n| n.decoratives.iter().map(|d| d.name.clone()).collect())
        .unwrap_or_default();
    let mut nearby = Vec::new();
    for dy in -1..=1 {
        for dx in -1..=1 {
            let n = ChunkPosition { x: c.x + dx, y: c.y + dy };
            nearby.extend(terrain.placed.entry(n).or_insert_with(|| sim.0.surface.decoratives(n)).iter().copied());
        }
    }
    for d in nearby {
        let Some(name) = names.get(d.decorative as usize) else { continue };
        let t = &mut *terrain;
        let files = &mut t.files;
        let pic = t.levels.entry(px).or_default().decoratives.entry((d.decorative, d.variation)).or_insert_with(|| {
            let s = factorio_data::sprite::decorative_sprite(&data.0, name, d.variation as usize)?;
            let img = load_file(files, &s.path)?;
            if s.x + s.width > img.width() || s.y + s.height > img.height() {
                return None;
            }
            let k = s.scale as f32 * px as f32 / 32.0;
            let (w, h) = (((s.width as f32 * k).round() as u32).max(1), ((s.height as f32 * k).round() as u32).max(1));
            let crop = image::imageops::crop_imm(img, s.x, s.y, s.width, s.height).to_image();
            let small = image::imageops::resize(&crop, w, h, image::imageops::FilterType::CatmullRom);
            let ox = (s.shift.0 as f32 * px as f32) as i32 - w as i32 / 2;
            let oy = (s.shift.1 as f32 * px as f32) as i32 - h as i32 / 2;
            Some((small, ox, oy))
        });
        let Some((pic, ox, oy)) = pic else { continue };
        let x0 = (d.x - first.x * 256) * px as i32 / 256 + *ox;
        let y0 = (d.y - first.y * 256) * px as i32 / 256 + *oy;
        for (x, y, p) in pic.enumerate_pixels() {
            let (dx, dy) = (x0 + x as i32, y0 + y as i32);
            if dx < 0 || dy < 0 || dx >= size as i32 || dy >= size as i32 || p[3] == 0 {
                continue;
            }
            let i = ((dy as u32 * size + dx as u32) * 4) as usize;
            let a = p[3] as u32;
            for ch in 0..3 {
                pixels[i + ch] = ((pixels[i + ch] as u32 * (255 - a) + p[ch] as u32 * a) / 255) as u8;
            }
        }
    }
    true
}

fn chunk_sprite(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    c: ChunkPosition,
    pixels: Vec<u8>,
    px: u32,
    z: f32,
) -> (Entity, Handle<Image>) {
    let size = CHUNK_SIZE as u32 * px;
    let image = Image::new(
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    let handle = images.add(image);
    let first = c.first_tile();
    let world = crate::map_to_world(MapPosition::from_tiles(first.x, first.y))
        + Vec2::new(1.0, -1.0) * (CHUNK_SIZE as f32 * TILE / 2.0);
    let e = commands
        .spawn((
            Sprite { image: handle.clone(), custom_size: Some(Vec2::splat(CHUNK_SIZE as f32 * TILE)), ..default() },
            Transform::from_xyz(world.x, world.y, z),
        ))
        .id();
    (e, handle)
}

fn build_chunks(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    mut terrain: ResMut<Terrain>,
    mut ready: ResMut<GroundReady>,
    mut images: ResMut<Assets<Image>>,
    camera: Single<(&Transform, &Projection), With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut visibility: Query<&mut Visibility>,
) {
    let started = std::time::Instant::now();
    let (ct, proj) = *camera;
    let scale = match proj {
        Projection::Orthographic(o) => o.scale,
        _ => 1.0,
    };
    // The detail the view needs: the game's full 64 px art at normal zoom and closer.
    let want = if scale <= 1.05 {
        HI
    } else if scale <= MID_MAX_SCALE {
        MID
    } else {
        LO
    };
    let half = window.size() / 2.0 * scale;
    let centre = ct.translation.truncate();
    let chunk_world = CHUNK_SIZE as f32 * TILE;
    // Chunks in view, plus one around it.
    let lt = crate::world_to_map(Vec2::new(centre.x - half.x, centre.y + half.y)).tile().chunk();
    let rb = crate::world_to_map(Vec2::new(centre.x + half.x, centre.y - half.y)).tile().chunk();
    let surface = &sim.0.surface;
    let mut wanted: Vec<(f32, ChunkPosition)> = Vec::new();
    for y in lt.y - 1..=rb.y + 1 {
        for x in lt.x - 1..=rb.x + 1 {
            let c = ChunkPosition { x, y };
            // Drawn once its neighbours exist, so its edges blend into them.
            let neighbours =
                (-1..=1).all(|dy| (-1..=1).all(|dx| surface.is_generated(ChunkPosition { x: x + dx, y: y + dy })));
            if !neighbours {
                continue;
            }
            let mid = crate::map_to_world(MapPosition::from_tiles(
                c.x * CHUNK_SIZE + CHUNK_SIZE / 2,
                c.y * CHUNK_SIZE + CHUNK_SIZE / 2,
            ));
            wanted.push((mid.distance(centre) / chunk_world, c));
        }
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let in_view: HashSet<ChunkPosition> = wanted.iter().map(|w| w.1).collect();

    // Work on composing textures: small ones first everywhere (they fill the view fast),
    // then the needed detail nearest first, a row at a time within the frame budget.
    while started.elapsed().as_secs_f64() * 1000.0 < BUDGET_MS {
        if terrain.job.is_none() {
            let next = wanted
                .iter()
                .find(|(_, c)| !terrain.chunks.get(c).is_some_and(|v| v.levels.contains_key(&LO)))
                .map(|(_, c)| (*c, LO))
                .or_else(|| {
                    wanted
                        .iter()
                        .find(|(_, c)| !terrain.chunks.get(c).is_some_and(|v| v.levels.contains_key(&want)))
                        .map(|(_, c)| (*c, want))
                });
            match next {
                Some((c, px)) => terrain.job = Some(Job::new(c, px)),
                None => break,
            }
        }
        let mut job = terrain.job.take().unwrap();
        if !in_view.contains(&job.c) {
            continue;
        }
        if compose_step(&mut terrain, &sim, &data, &mut job) {
            let z = -100.0 - (job.px == LO) as i32 as f32;
            let sprite = chunk_sprite(&mut commands, &mut images, job.c, std::mem::take(&mut job.pixels), job.px, z);
            terrain.chunks.entry(job.c).or_default().levels.insert(job.px, sprite);
            ready.0.insert(job.c);
        } else {
            terrain.job = Some(job);
        }
    }

    // Keep the small textures; drop detail the view no longer needs.
    for (c, view) in terrain.chunks.iter_mut() {
        let drop: Vec<u32> =
            view.levels.keys().copied().filter(|px| *px != LO && (*px != want || !in_view.contains(c))).collect();
        for px in drop {
            if let Some((e, h)) = view.levels.remove(&px) {
                commands.entity(e).despawn();
                images.remove(&h);
            }
        }
        // Show the wanted detail if it is ready, else the small texture.
        let shown = if view.levels.contains_key(&want) { want } else { LO };
        for (px, (e, _)) in &view.levels {
            if let Ok(mut v) = visibility.get_mut(*e) {
                *v = if *px == shown { Visibility::Inherited } else { Visibility::Hidden };
            }
        }
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
