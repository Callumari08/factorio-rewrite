//! Draws the ground on the GPU, like the game: the tile, transition, shore and decorative
//! sheets from the install are loaded once with full mipmaps (sampled trilinearly), and
//! each chunk gets static meshes of quads pointing into them. Zooming only moves the
//! camera; the mipmaps give the right detail at any zoom with nothing recomposed.
//!
//! Tiles use their 4x4 and 2x2 pictures where an aligned block is one tile (as the game
//! does), else a 1x1 variant. Where a tile borders one of a higher `layer`, the higher
//! tile is drawn into it through the pieces of its transition mask sheet (a small shader
//! multiplies the picture by the mask). Shores use the land tiles' `transitions` to
//! water: background, mask and bank overlay pieces, up to two tiles tall. Water meets deep
//! water through a soft mask made at startup. Resource deposits are sprites on top.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use bevy::asset::{RenderAssetUsages, uuid_handle};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};
use factorio_sim::map::{CHUNK_SIZE, ChunkPosition, MapPosition, TilePosition};
use factorio_sim::noise::hash;
use factorio_sim::proto::{EntityData, EntityProtoId, TileId};

use crate::sprites::Sprites;
use crate::{Data, Sim, TILE};

const MASKED_SHADER: Handle<Shader> = uuid_handle!("6f0b5a4e-2f3c-4d0a-9a51-1c7e0c3b8d21");

/// How far (in chunks) around the camera ground meshes are kept.
const KEEP_CHUNKS: i32 = 12;
/// Chunks meshed per frame at most.
const BUILD_PER_FRAME: usize = 8;

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::load_internal_asset!(app, MASKED_SHADER, "masked_tile.wgsl", Shader::from_wgsl);
        app.add_plugins(Material2dPlugin::<MaskedTile>::default())
            .init_resource::<Terrain>()
            .init_resource::<GroundReady>()
            .add_systems(Update, (build_chunks, update_resources).chain());
    }
}

/// A ground picture, optionally drawn through a mask (see `masked_tile.wgsl`).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct MaskedTile {
    #[texture(0)]
    #[sampler(1)]
    color: Handle<Image>,
    #[texture(2)]
    #[sampler(3)]
    mask: Handle<Image>,
}

impl Material2d for MaskedTile {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(MASKED_SHADER)
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(MASKED_SHADER)
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

/// Chunks whose ground is drawn; entities on other chunks are not drawn yet.
#[derive(Resource, Default)]
pub struct GroundReady(pub HashSet<ChunkPosition>);

#[derive(Default)]
struct ChunkView {
    scanned: bool,
    resources: HashMap<TilePosition, (Entity, EntityProtoId, u32)>,
    /// Whether the ground is built: drawn by `ground`, or by its region's meshes.
    built: bool,
    /// The ground's quads, kept until its region is merged.
    parts: Option<Parts>,
    ground: Vec<Entity>,
}

/// A loaded sheet: its texture (with mipmaps) and size in pixels.
#[derive(Clone)]
struct Sheet {
    image: Handle<Image>,
    size: Vec2,
}

#[derive(Resource, Default)]
struct Terrain {
    chunks: HashMap<ChunkPosition, ChunkView>,
    /// Merged ground meshes by region.
    regions: HashMap<(i32, i32), Region>,
    sheets: HashMap<(PathBuf, bool), Option<Sheet>>,
    white: Option<Handle<Image>>,
    masked: HashMap<(Handle<Image>, Handle<Image>), Handle<MaskedTile>>,
    /// Tile pictures by (tile, block size): sheet and pixel rects of the variants.
    blocks: HashMap<(TileId, u32), Option<(PathBuf, Vec<Rect>)>>,
    transitions: HashMap<TileId, Option<factorio_data::sprite::TileTransition>>,
    shores: HashMap<TileId, Vec<factorio_data::sprite::Shore>>,
    decorative_sprites: HashMap<(u16, u8), Option<factorio_data::sprite::SpriteRef>>,
    soft: Option<Sheet>,
    frame: u32,
    ground_frame: u32,
}

/// The texture's mip levels (box-filtered) concatenated, and how many there are.
fn mip_chain(level0: Vec<u8>, width: u32, height: u32) -> (Vec<u8>, u32) {
    let mut data = level0.clone();
    let mut prev = level0;
    let (mut w, mut h) = (width, height);
    let mut levels = 1;
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for ch in 0..4 {
                    let at = |xx: u32, yy: u32| prev[((yy.min(h - 1) * w + xx.min(w - 1)) * 4 + ch) as usize] as u32;
                    let sum = at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1);
                    next[((y * nw + x) * 4 + ch) as usize] = ((sum + 2) / 4) as u8;
                }
            }
        }
        data.extend_from_slice(&next);
        prev = next;
        (w, h) = (nw, nh);
        levels += 1;
    }
    (data, levels)
}

/// An RGBA image with a full mip chain and trilinear sampling. Masks are linear data.
fn mipmapped(pixels: Vec<u8>, width: u32, height: u32, linear: bool) -> Image {
    let (data, levels) = mip_chain(pixels, width, height);
    let format = if linear { TextureFormat::Rgba8Unorm } else { TextureFormat::Rgba8UnormSrgb };
    let mut image = Image::new_fill(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..bevy::image::ImageSamplerDescriptor::linear()
    });
    image
}

impl Terrain {
    fn sheet(&mut self, images: &mut Assets<Image>, path: &Path, linear: bool) -> Option<Sheet> {
        self.sheets
            .entry((path.to_owned(), linear))
            .or_insert_with(|| {
                let img = image::open(path).ok()?.to_rgba8();
                let (w, h) = img.dimensions();
                let handle = images.add(mipmapped(img.into_raw(), w, h, linear));
                Some(Sheet { image: handle, size: Vec2::new(w as f32, h as f32) })
            })
            .clone()
    }

    /// A 1×1 white mask, for pictures drawn without one.
    fn white(&mut self, images: &mut Assets<Image>) -> Handle<Image> {
        self.white.get_or_insert_with(|| images.add(mipmapped(vec![255; 4], 1, 1, true))).clone()
    }

    fn masked_material(
        &mut self,
        materials: &mut Assets<MaskedTile>,
        color: &Handle<Image>,
        mask: &Handle<Image>,
    ) -> Handle<MaskedTile> {
        self.masked
            .entry((color.clone(), mask.clone()))
            .or_insert_with(|| materials.add(MaskedTile { color: color.clone(), mask: mask.clone() }))
            .clone()
    }

    /// The pictures of a tile's `size`×`size` blocks: their sheet and variant rects.
    fn blocks(&mut self, data: &Data, sim: &Sim, tile: TileId, size: u32) -> Option<(PathBuf, Vec<Rect>)> {
        self.blocks
            .entry((tile, size))
            .or_insert_with(|| {
                let name = &sim.0.prototypes().tile(tile).name;
                let (_, sprites) =
                    factorio_data::sprite::tile_variant_sets(&data.0, name).into_iter().find(|(s, _)| *s == size)?;
                let path = sprites.first()?.path.clone();
                let rects = sprites
                    .iter()
                    .filter(|v| v.path == path)
                    .map(|v| Rect::new(v.x as f32, v.y as f32, (v.x + v.width) as f32, (v.y + v.height) as f32))
                    .collect();
                Some((path, rects))
            })
            .clone()
    }

    /// A 64 px soft mask sheet for water meeting deep water: column 0 the four sides,
    /// column 1 the four corners (rotations N/NE, E/SE, S/SW, W/NW).
    fn soft(&mut self, images: &mut Assets<Image>) -> Sheet {
        if let Some(s) = &self.soft {
            return s.clone();
        }
        let px = 64u32;
        let (w, h) = (2 * px, 4 * px);
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for rot in 0..4 {
            for y in 0..px {
                for x in 0..px {
                    let (fx, fy) = ((x as f32 + 0.5) / px as f32, (y as f32 + 0.5) / px as f32);
                    let side = [fy, 1.0 - fx, 1.0 - fy, fx][rot as usize];
                    let corner = [(1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)][rot as usize];
                    let cd = ((fx - corner.0).powi(2) + (fy - corner.1).powi(2)).sqrt();
                    for (col, d) in [(0u32, side), (1, cd)] {
                        let t = (1.0f32 - d).clamp(0.0, 1.0);
                        let v = (t * t * (3.0 - 2.0 * t) * 255.0) as u8;
                        let i = (((rot * px + y) * w + col * px + x) * 4) as usize;
                        pixels[i..i + 4].copy_from_slice(&[v, v, v, 255]);
                    }
                }
            }
        }
        let s = Sheet { image: images.add(mipmapped(pixels, w, h, true)), size: Vec2::new(w as f32, h as f32) };
        self.soft = Some(s.clone());
        s
    }
}

/// Quads for one mesh: positions in world units, UVs, and an optional second UV (the mask)
/// carried in the vertex colour.
#[derive(Default, Clone)]
struct Quads {
    pos: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    color_rect: Vec<[f32; 4]>,
    mask_rect: Vec<[f32; 4]>,
    idx: Vec<u32>,
}

/// UV rect of a pixel rect (the shader keeps samples inside it).
fn uv_rect(r: Rect, size: Vec2) -> Rect {
    Rect::new(r.min.x / size.x, r.min.y / size.y, r.max.x / size.x, r.max.y / size.y)
}

/// The whole of the white mask (for unmasked ground pictures).
const NO_MASK: Rect = Rect { min: Vec2::ZERO, max: Vec2::ONE };

impl Quads {
    /// A quad covering tiles from (x, y) (top left) for w×h tiles.
    fn add(&mut self, x: f32, y: f32, w: f32, h: f32, uv: Rect, mask: Option<Rect>) {
        let base = self.pos.len() as u32;
        let (x0, y0, x1, y1) = (x * TILE, -y * TILE, (x + w) * TILE, -(y + h) * TILE);
        self.pos.extend([[x0, y0, 0.0], [x1, y0, 0.0], [x1, y1, 0.0], [x0, y1, 0.0]]);
        self.uv.extend([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        self.color_rect.extend([[uv.min.x, uv.min.y, uv.max.x, uv.max.y]; 4]);
        let m = mask.unwrap_or(NO_MASK);
        self.mask_rect.extend([[m.min.x, m.min.y, m.max.x, m.max.y]; 4]);
        self.idx.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }

    fn mesh(self) -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uv)
            .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, self.color_rect)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.mask_rect)
            .with_inserted_indices(Indices::U32(self.idx))
    }
}

/// Mesh parts of a chunk, by draw order and material.
#[derive(Default, Clone)]
struct Parts {
    plain: HashMap<(i32, Handle<Image>), Quads>,
    masked: HashMap<(i32, Handle<Image>, Handle<Image>), Quads>,
}

impl Quads {
    fn append(&mut self, other: Quads) {
        let base = self.pos.len() as u32;
        self.pos.extend(other.pos);
        self.uv.extend(other.uv);
        self.color_rect.extend(other.color_rect);
        self.mask_rect.extend(other.mask_rect);
        self.idx.extend(other.idx.into_iter().map(|i| i + base));
    }
}

impl Parts {
    fn append(&mut self, other: Parts) {
        for (k, q) in other.plain {
            self.plain.entry(k).or_default().append(q);
        }
        for (k, q) in other.masked {
            self.masked.entry(k).or_default().append(q);
        }
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

/// Draw order of ground layers (higher is on top): base tiles, then transitions by the
/// higher tile's layer, shores, decoratives.
const ORDER_BASE: i32 = 0;
const ORDER_TRANSITION: i32 = 1000;
const ORDER_SHORE_BACKGROUND: i32 = 3000;
const ORDER_SHORE_MASK: i32 = 3001;
const ORDER_SHORE_OVERLAY: i32 = 3002;
const ORDER_DECORATIVE: i32 = 4000;

fn build_parts(terrain: &mut Terrain, sim: &Sim, data: &Data, images: &mut Assets<Image>, c: ChunkPosition) -> Parts {
    let db = sim.0.prototypes();
    let surface = &sim.0.surface;
    let first = c.first_tile();
    let mut parts = Parts::default();
    // Base tiles, biggest blocks first.
    let mut covered = vec![false; (CHUNK_SIZE * CHUNK_SIZE) as usize];
    for size in [4i32, 2, 1] {
        for ly in (0..CHUNK_SIZE).step_by(size as usize) {
            for lx in (0..CHUNK_SIZE).step_by(size as usize) {
                let t = TilePosition::new(first.x + lx, first.y + ly);
                let Some(tile) = surface.tile(t) else { continue };
                let cells = (0..size).flat_map(|dy| (0..size).map(move |dx| (lx + dx, ly + dy)));
                let whole = cells.clone().all(|(x, y)| {
                    !covered[(y * CHUNK_SIZE + x) as usize]
                        && surface.tile(TilePosition::new(first.x + x, first.y + y)) == Some(tile)
                });
                if !whole {
                    continue;
                }
                let Some((path, rects)) = terrain.blocks(data, sim, tile, size as u32) else { continue };
                if rects.is_empty() {
                    continue;
                }
                let Some(sheet) = terrain.sheet(images, &path, false) else { continue };
                let r = rects[hash(7, 10 + size as u32, t.x, t.y) as usize % rects.len()];
                parts.plain.entry((ORDER_BASE, sheet.image.clone())).or_default().add(
                    t.x as f32,
                    t.y as f32,
                    size as f32,
                    size as f32,
                    uv_rect(r, sheet.size),
                    None,
                );
                for (x, y) in cells {
                    covered[(y * CHUNK_SIZE + x) as usize] = true;
                }
            }
        }
    }
    // Transitions and shores.
    for ly in 0..CHUNK_SIZE {
        for lx in 0..CHUNK_SIZE {
            let t = TilePosition::new(first.x + lx, first.y + ly);
            let Some(tile) = surface.tile(t) else { continue };
            let around = [(0, -1), (1, 0), (0, 1), (-1, 0), (1, -1), (1, 1), (-1, 1), (-1, -1)]
                .map(|(dx, dy)| surface.tile(TilePosition::new(t.x + dx, t.y + dy)));
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
                let edges = [0, 1, 2, 3].map(|i| around[i] == Some(over));
                let corners = [4, 5, 6, 7].map(|i| around[i] == Some(over));
                let pieces = pick_pieces(edges, corners);
                if pieces.is_empty() {
                    continue;
                }
                let pick = hash(7, 2 + over.0 as u32, t.x, t.y) as u32;
                // The higher tile's own 1x1 picture, sampled through the mask.
                let Some((cpath, crects)) = terrain.blocks(data, sim, over, 1) else { continue };
                let Some(csheet) = terrain.sheet(images, &cpath, false) else { continue };
                if crects.is_empty() {
                    continue;
                }
                let color_uv = uv_rect(crects[hash(7, 1, t.x, t.y) as usize % crects.len()], csheet.size);
                let over_name = db.tile(over).name.clone();
                // Shores: the land's own transition to this (water) tile.
                let shores = terrain
                    .shores
                    .entry(over)
                    .or_insert_with(|| factorio_data::sprite::tile_shores(&data.0, &over_name))
                    .clone();
                if let Some(shore) = shores.iter().find(|s| s.to_tiles.iter().any(|n| *n == db.tile(tile).name)) {
                    let Some(sheet) = terrain.sheet(images, &shore.sheet, false) else { continue };
                    let Some(mask_sheet) = terrain.sheet(images, &shore.sheet, true) else { continue };
                    let sz = shore.size as f32;
                    for (kind, rot) in &pieces {
                        let p = match kind {
                            Piece::Inner => shore.inner_corner,
                            Piece::Outer => shore.outer_corner,
                            Piece::Side => shore.side,
                            Piece::U => shore.u_transition,
                            Piece::O => shore.o_transition,
                        };
                        if p.count == 0 {
                            continue;
                        }
                        let v = (pick % p.count) as f32;
                        let h = p.tile_height.max(1) as f32;
                        let rot = if *kind == Piece::O { 0.0 } else { *rot as f32 };
                        let tall = |x0: u32| {
                            Rect::new(
                                x0 as f32 + v * sz,
                                p.y as f32 + rot * sz * h,
                                x0 as f32 + (v + 1.0) * sz,
                                p.y as f32 + (rot + 1.0) * sz * h,
                            )
                        };
                        if let Some(bx) = shore.background_x {
                            parts.plain.entry((ORDER_SHORE_BACKGROUND, sheet.image.clone())).or_default().add(
                                t.x as f32,
                                t.y as f32,
                                1.0,
                                h,
                                uv_rect(tall(bx), sheet.size),
                                None,
                            );
                        }
                        let mask = Rect::new(
                            shore.mask_x as f32 + v * sz,
                            p.y as f32 + rot * sz,
                            shore.mask_x as f32 + (v + 1.0) * sz,
                            p.y as f32 + (rot + 1.0) * sz,
                        );
                        parts
                            .masked
                            .entry((ORDER_SHORE_MASK, csheet.image.clone(), mask_sheet.image.clone()))
                            .or_default()
                            .add(t.x as f32, t.y as f32, 1.0, 1.0, color_uv, Some(uv_rect(mask, mask_sheet.size)));
                        parts.plain.entry((ORDER_SHORE_OVERLAY, sheet.image.clone())).or_default().add(
                            t.x as f32,
                            t.y as f32,
                            1.0,
                            h,
                            uv_rect(tall(shore.overlay_x), sheet.size),
                            None,
                        );
                    }
                    continue;
                }
                let order = ORDER_TRANSITION + db.tile(over).layer.clamp(0, 999);
                if water && db.tile(over).fluid.is_some() {
                    // Water into deep water: soft sides and corners.
                    let soft = terrain.soft(images);
                    for (i, e) in edges.iter().enumerate() {
                        if *e {
                            let m = Rect::new(0.0, i as f32 * 64.0, 64.0, (i + 1) as f32 * 64.0);
                            parts.masked.entry((order, csheet.image.clone(), soft.image.clone())).or_default().add(
                                t.x as f32,
                                t.y as f32,
                                1.0,
                                1.0,
                                color_uv,
                                Some(uv_rect(m, soft.size)),
                            );
                        }
                    }
                    for (i, k) in corners.iter().enumerate() {
                        if *k && !edges[i] && !edges[(i + 1) % 4] {
                            let m = Rect::new(64.0, i as f32 * 64.0, 128.0, (i + 1) as f32 * 64.0);
                            parts.masked.entry((order, csheet.image.clone(), soft.image.clone())).or_default().add(
                                t.x as f32,
                                t.y as f32,
                                1.0,
                                1.0,
                                color_uv,
                                Some(uv_rect(m, soft.size)),
                            );
                        }
                    }
                    continue;
                }
                let tr = terrain
                    .transitions
                    .entry(over)
                    .or_insert_with(|| factorio_data::sprite::tile_transition(&data.0, &over_name))
                    .clone();
                let Some(tr) = tr else { continue };
                let Some(mask_sheet) = terrain.sheet(images, &tr.sheet, true) else { continue };
                let sz = tr.size as f32;
                for (kind, rot) in pieces {
                    let p = match kind {
                        Piece::Inner => tr.inner_corner,
                        Piece::Outer => tr.outer_corner,
                        Piece::Side => tr.side,
                        Piece::U => tr.u_transition,
                        Piece::O => tr.o_transition,
                    };
                    if p.count == 0 {
                        continue;
                    }
                    let v = (pick % p.count) as f32;
                    let rot = if kind == Piece::O { 0.0 } else { rot as f32 };
                    let m = Rect::new(
                        p.x as f32 + v * sz,
                        tr.y as f32 + rot * sz,
                        p.x as f32 + (v + 1.0) * sz,
                        tr.y as f32 + (rot + 1.0) * sz,
                    );
                    parts.masked.entry((order, csheet.image.clone(), mask_sheet.image.clone())).or_default().add(
                        t.x as f32,
                        t.y as f32,
                        1.0,
                        1.0,
                        color_uv,
                        Some(uv_rect(m, mask_sheet.size)),
                    );
                }
            }
        }
    }
    // Decoratives.
    let names: Vec<String> = surface
        .settings
        .noise
        .as_ref()
        .map(|n| n.decoratives.iter().map(|d| d.name.clone()).collect())
        .unwrap_or_default();
    for d in surface.decoratives(c) {
        let Some(name) = names.get(d.decorative as usize) else { continue };
        let s = terrain
            .decorative_sprites
            .entry((d.decorative, d.variation))
            .or_insert_with(|| factorio_data::sprite::decorative_sprite(&data.0, name, d.variation as usize))
            .clone();
        let Some(s) = s else { continue };
        let Some(sheet) = terrain.sheet(images, &s.path, false) else { continue };
        let (w, h) = (s.width as f32 * s.scale as f32 / 32.0, s.height as f32 * s.scale as f32 / 32.0);
        let (cx, cy) = (d.x as f32 / 256.0 + s.shift.0 as f32, d.y as f32 / 256.0 + s.shift.1 as f32);
        let r = Rect::new(s.x as f32, s.y as f32, (s.x + s.width) as f32, (s.y + s.height) as f32);
        parts.plain.entry((ORDER_DECORATIVE, sheet.image.clone())).or_default().add(
            cx - w / 2.0,
            cy - h / 2.0,
            w,
            h,
            uv_rect(r, sheet.size),
            None,
        );
    }
    parts
}

/// Finished chunks are merged into meshes covering `REGION`×`REGION` chunks, so the
/// ground takes a few draws per sheet rather than one per chunk.
const REGION: i32 = 8;

fn region_of(c: ChunkPosition) -> (i32, i32) {
    (c.x.div_euclid(REGION), c.y.div_euclid(REGION))
}

fn region_chunks(r: (i32, i32)) -> impl Iterator<Item = ChunkPosition> {
    (0..REGION)
        .flat_map(move |dy| (0..REGION).map(move |dx| ChunkPosition { x: r.0 * REGION + dx, y: r.1 * REGION + dy }))
}

/// Frames without a new chunk before a region's meshes are merged again.
const MERGE_DELAY_FRAMES: u32 = 30;

#[derive(Default)]
struct Region {
    ground: Vec<Entity>,
    /// The frame a chunk of the region was last built.
    touched: u32,
}

/// Spawns the meshes of some ground parts.
fn spawn_parts(
    commands: &mut Commands,
    terrain: &mut Terrain,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    masked: &mut Assets<MaskedTile>,
    parts: Parts,
) -> Vec<Entity> {
    let white = terrain.white(images);
    let plain = parts.plain.into_iter().map(|((order, image), q)| ((order, image, white.clone()), q));
    let mut out = Vec::new();
    for ((order, color, mask), quads) in plain.chain(parts.masked) {
        let material = terrain.masked_material(masked, &color, &mask);
        let z = -100.0 + order as f32 * 0.001;
        out.push(
            commands
                .spawn((Mesh2d(meshes.add(quads.mesh())), MeshMaterial2d(material), Transform::from_xyz(0.0, 0.0, z)))
                .id(),
        );
    }
    out
}

fn build_chunks(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    mut terrain: ResMut<Terrain>,
    mut ready: ResMut<GroundReady>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut masked: ResMut<Assets<MaskedTile>>,
    camera: Single<&Transform, With<Camera2d>>,
) {
    let centre = camera.translation.truncate();
    let at = crate::world_to_map(centre).tile().chunk();
    let surface = &sim.0.surface;
    let t = &mut *terrain;
    t.ground_frame += 1;
    // Generated chunks near the camera whose neighbours exist (so edges can blend).
    let mut todo: Vec<(i32, ChunkPosition)> = surface
        .chunks()
        .map(|(p, _)| p)
        .filter(|p| !t.chunks.get(p).is_some_and(|v| v.built))
        .filter(|p| (p.x - at.x).abs() <= KEEP_CHUNKS && (p.y - at.y).abs() <= KEEP_CHUNKS)
        .filter(|p| {
            (-1..=1).all(|dy| (-1..=1).all(|dx| surface.is_generated(ChunkPosition { x: p.x + dx, y: p.y + dy })))
        })
        .map(|p| ((p.x - at.x).abs().max((p.y - at.y).abs()), p))
        .collect();
    todo.sort();
    for (_, c) in todo.into_iter().take(BUILD_PER_FRAME) {
        let parts = build_parts(t, &sim, &data, &mut images, c);
        // Drawn on its own until its region is merged again.
        let ground = spawn_parts(&mut commands, t, &mut images, &mut meshes, &mut masked, parts.clone());
        let view = t.chunks.entry(c).or_default();
        view.built = true;
        view.parts = Some(parts);
        view.ground = ground;
        t.regions.entry(region_of(c)).or_default().touched = t.ground_frame;
        ready.0.insert(c);
    }
    // Regions whose chunks stopped changing are merged into one set of meshes.
    let settled: Vec<(i32, i32)> = t
        .regions
        .iter()
        .filter(|(r, g)| {
            t.ground_frame > g.touched + MERGE_DELAY_FRAMES
                && region_chunks(**r).any(|c| t.chunks.get(&c).is_some_and(|v| !v.ground.is_empty()))
        })
        .map(|(r, _)| *r)
        .collect();
    for r in settled {
        let mut merged = Parts::default();
        let mut full = true;
        for c in region_chunks(r) {
            let Some(v) = t.chunks.get_mut(&c).filter(|v| v.built) else {
                full = false;
                continue;
            };
            for e in v.ground.drain(..) {
                commands.entity(e).despawn();
            }
            if let Some(p) = &v.parts {
                merged.append(p.clone());
            }
        }
        // A full region never changes again, so its chunks' quads are not needed.
        if full {
            for c in region_chunks(r) {
                t.chunks.get_mut(&c).unwrap().parts = None;
            }
        }
        let ground = spawn_parts(&mut commands, t, &mut images, &mut meshes, &mut masked, merged);
        for e in std::mem::replace(&mut t.regions.get_mut(&r).unwrap().ground, ground) {
            commands.entity(e).despawn();
        }
    }
    // Far regions drop their meshes.
    let far = |c: ChunkPosition| (c.x - at.x).abs() > KEEP_CHUNKS + REGION || (c.y - at.y).abs() > KEEP_CHUNKS + REGION;
    let far_regions: Vec<(i32, i32)> = t.regions.keys().copied().filter(|r| region_chunks(*r).all(far)).collect();
    for r in far_regions {
        for e in t.regions.remove(&r).unwrap().ground {
            commands.entity(e).despawn();
        }
        for c in region_chunks(r) {
            if let Some(v) = t.chunks.get_mut(&c) {
                for e in v.ground.drain(..) {
                    commands.entity(e).despawn();
                }
                v.built = false;
                v.parts = None;
            }
            ready.0.remove(&c);
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
