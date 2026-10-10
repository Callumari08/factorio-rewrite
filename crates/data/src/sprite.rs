//! Finding sprite images referenced by prototypes. Rendering-side data, so floats are fine.

use std::path::PathBuf;

use crate::datastage::GameData;
use crate::raw::RawValue;

#[derive(Clone, Debug, PartialEq)]
pub struct SpriteRef {
    /// Absolute path to the PNG inside the user's install.
    pub path: PathBuf,
    /// Region of the image to draw, in pixels.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Factorio `scale`: 0.5 for the high-resolution sprites used throughout 2.0.
    pub scale: f64,
    /// Offset of the sprite centre from the entity position, in tiles (Factorio `shift`, y down).
    pub shift: (f64, f64),
}

/// The icon of an item-category prototype (`icon`, or the first layer of `icons`).
/// Icons in 2.0 are stored with mipmaps side by side; only the full-size one is returned.
pub fn item_icon(data: &GameData, name: &str) -> Option<SpriteRef> {
    let (_, _, proto) = data.prototypes_in_category("item").find(|(_, n, _)| *n == name)?;
    icon_of(data, proto)
}

pub fn icon_of(data: &GameData, proto: &RawValue) -> Option<SpriteRef> {
    let (layer, size_from) = match proto.get("icon").as_str() {
        Some(_) => (proto, proto),
        None => (proto.get("icons").at(0), proto.get("icons").at(0)),
    };
    let size = size_from.get("icon_size").as_i64().or(proto.get("icon_size").as_i64()).unwrap_or(64) as u32;
    Some(SpriteRef {
        path: data.resolve_path(layer.get("icon").as_str()?)?,
        x: 0,
        y: 0,
        width: size,
        height: size,
        scale: 1.0,
        shift: (0.0, 0.0),
    })
}

/// The first still sprite found in an entity prototype's graphics, searched depth-first
/// through the usual graphics keys. Good enough for previews until real entity renderers exist.
pub fn entity_sprite(data: &GameData, name: &str) -> Option<SpriteRef> {
    let (_, _, proto) = data.prototypes_in_category("entity").find(|(_, n, _)| *n == name)?;
    let keys = [
        "picture",
        "pictures",
        "animation",
        "structure",
        "graphics_set",
        "platform_picture",
        "belt_animation_set",
        "integration_patch",
    ];
    for key in keys {
        if let Some(s) = find_sprite(data, proto.get(key), 0) {
            return Some(s);
        }
    }
    None
}

fn find_sprite(data: &GameData, v: &RawValue, depth: usize) -> Option<SpriteRef> {
    if depth > 8 {
        return None;
    }
    match v {
        RawValue::Table(t) => {
            // Size is `width`/`height`, or `size` as a number or `{w, h}` pair.
            let size = v.get("size");
            let w = v.get("width").as_i64().or(size.as_i64()).or(size.at(0).as_i64());
            let h = v.get("height").as_i64().or(size.as_i64()).or(size.at(1).as_i64());
            if let (Some(file), Some(w), Some(h)) = (v.get("filename").as_str(), w, h) {
                return Some(SpriteRef {
                    path: data.resolve_path(file)?,
                    x: v.get("x").as_i64().unwrap_or(0) as u32,
                    y: v.get("y").as_i64().unwrap_or(0) as u32,
                    width: w as u32,
                    height: h as u32,
                    scale: v.get("scale").as_f64().unwrap_or(1.0),
                    shift: vector(v.get("shift")),
                });
            }
            // Prefer the main layer / north-facing variant when present.
            for key in ["layers", "sheet", "sheets", "north", "animation", "picture", "structure"] {
                if let Some(s) = find_sprite(data, v.get(key), depth + 1) {
                    return Some(s);
                }
            }
            t.values().find_map(|c| find_sprite(data, c, depth + 1))
        }
        RawValue::Array(a) => a.iter().find_map(|c| find_sprite(data, c, depth + 1)),
        _ => None,
    }
}

fn vector(v: &RawValue) -> (f64, f64) {
    let x = v.at(0).as_f64().or(v.get("x").as_f64()).unwrap_or(0.0);
    let y = v.at(1).as_f64().or(v.get("y").as_f64()).unwrap_or(0.0);
    (x, y)
}

fn sprite_from(data: &GameData, v: &RawValue, frame: u32, row: u32) -> Option<SpriteRef> {
    let file = v.get("filename").as_str()?;
    let size = v.get("size");
    let w = v.get("width").as_i64().or(size.as_i64()).or(size.at(0).as_i64())? as u32;
    let h = v.get("height").as_i64().or(size.as_i64()).or(size.at(1).as_i64())? as u32;
    let line_length = v.get("line_length").as_i64().filter(|l| *l > 0).map(|l| l as u32);
    let (col, extra_row) = match line_length {
        Some(l) => (frame % l, frame / l),
        None => (frame, 0),
    };
    Some(SpriteRef {
        path: data.resolve_path(file)?,
        x: v.get("x").as_i64().unwrap_or(0) as u32 + col * w,
        y: v.get("y").as_i64().unwrap_or(0) as u32 + (row + extra_row) * h,
        width: w,
        height: h,
        scale: v.get("scale").as_f64().unwrap_or(1.0),
        shift: vector(v.get("shift")),
    })
}

/// The first non-shadow layer of a sprite/animation definition, descending into
/// `layers`, `sheet` and `sheets`.
fn main_layer(v: &RawValue) -> Option<&RawValue> {
    if !v.get("layers").as_array().is_empty() {
        return v
            .get("layers")
            .as_array()
            .iter()
            .find(|l| l.get("draw_as_shadow").as_bool() != Some(true))
            .and_then(main_layer);
    }
    if !v.get("sheet").is_nil() {
        return main_layer(v.get("sheet"));
    }
    if let Some(s) = v.get("sheets").as_array().first() {
        return main_layer(s);
    }
    if !v.get("filename").is_nil() {
        return Some(v);
    }
    None
}

const DIR_KEYS: [&str; 4] = ["north", "east", "south", "west"];

/// Entity sprite for a cardinal direction (0 = north, 1 = east, ...). Handles the common
/// 2.0 layouts: per-direction tables (`{north = ..., east = ...}`), 4-way sheets (one frame
/// per direction) and plain single sprites.
pub fn entity_sprite_dir(data: &GameData, name: &str, dir: usize) -> Option<SpriteRef> {
    let (kind, _, proto) = data.prototypes_in_category("entity").find(|(_, n, _)| *n == name)?;
    if kind == "generator" {
        let key = if dir % 2 == 1 { "horizontal_animation" } else { "vertical_animation" };
        return main_layer(proto.get(key)).and_then(|l| sprite_from(data, l, 0, 0));
    }
    let keys =
        ["graphics_set", "picture", "pictures", "animation", "structure", "platform_picture", "integration_patch"];
    for key in keys {
        if let Some(s) = sprite_dir(data, proto.get(key), dir, 0) {
            return Some(s);
        }
    }
    None
}

fn sprite_dir(data: &GameData, v: &RawValue, dir: usize, depth: u32) -> Option<SpriteRef> {
    if depth > 6 || v.is_nil() {
        return None;
    }
    if !v.get(DIR_KEYS[0]).is_nil() {
        return sprite_dir(data, v.get(DIR_KEYS[dir]), dir, depth + 1);
    }
    if let Some(layer) = main_layer(v) {
        // A 4-way sheet has one frame per direction laid out horizontally.
        let four_way = !v.get("sheet").is_nil() && layer.get("frames").as_i64().is_none_or(|f| f >= 4);
        let direction_count = layer.get("direction_count").as_i64().unwrap_or(1);
        let (frame, row) = if four_way {
            (dir as u32, 0)
        } else if direction_count >= 4 {
            (0, (dir as i64 * direction_count / 4) as u32)
        } else {
            (0, 0)
        };
        return sprite_from(data, layer, frame, row);
    }
    for key in ["animation", "picture", "structure", "idle_animation", "working_visualisations"] {
        if let Some(s) = sprite_dir(data, v.get(key), dir, depth + 1) {
            return Some(s);
        }
    }
    if let RawValue::Array(a) = v {
        return a.iter().find_map(|x| sprite_dir(data, x, dir, depth + 1));
    }
    None
}

/// A named picture of a pipe-like entity (e.g. `straight_vertical`, `corner_up_right`).
pub fn pipe_picture(data: &GameData, name: &str, key: &str) -> Option<SpriteRef> {
    let (_, _, proto) = data.prototypes_in_category("entity").find(|(_, n, _)| *n == name)?;
    main_layer(proto.get("pictures").get(key)).and_then(|l| sprite_from(data, l, 0, 0))
}

/// Belt animation sheet: the sprite of row 0 / frame 0 plus frame count; rows are the
/// `*_index` values of `TransportBeltAnimationSet` (1-based).
#[derive(Clone, Debug)]
pub struct BeltSheet {
    pub base: SpriteRef,
    pub frame_count: u32,
    pub indices: std::collections::BTreeMap<String, u32>,
}

impl BeltSheet {
    pub fn frame(&self, index_name: &str, frame: u32) -> SpriteRef {
        let row = self.indices.get(index_name).copied().unwrap_or(1).saturating_sub(1);
        let mut s = self.base.clone();
        s.x += (frame % self.frame_count.max(1)) * s.width;
        s.y += row * s.height;
        s
    }
}

pub fn belt_sheet(data: &GameData, name: &str) -> Option<BeltSheet> {
    let (_, _, proto) = data.prototypes_in_category("entity").find(|(_, n, _)| *n == name)?;
    let set = proto.get("belt_animation_set");
    let anim = set.get("animation_set");
    let base = sprite_from(data, main_layer(anim)?, 0, 0)?;
    let defaults = [
        ("east_index", 1),
        ("west_index", 2),
        ("north_index", 3),
        ("south_index", 4),
        ("east_to_north_index", 5),
        ("north_to_east_index", 6),
        ("west_to_north_index", 7),
        ("north_to_west_index", 8),
        ("south_to_east_index", 9),
        ("east_to_south_index", 10),
        ("south_to_west_index", 11),
        ("west_to_south_index", 12),
        ("starting_south_index", 13),
        ("ending_south_index", 14),
        ("starting_west_index", 15),
        ("ending_west_index", 16),
        ("starting_north_index", 17),
        ("ending_north_index", 18),
        ("starting_east_index", 19),
        ("ending_east_index", 20),
    ];
    let indices =
        defaults.iter().map(|(k, d)| (k.to_string(), set.get(k).as_i64().map(|v| v as u32).unwrap_or(*d))).collect();
    Some(BeltSheet { base, frame_count: anim.get("frame_count").as_i64().unwrap_or(1) as u32, indices })
}

/// Inserter hand graphics (open hand, base of the arm).
pub fn inserter_hand(data: &GameData, name: &str) -> Option<(SpriteRef, SpriteRef)> {
    let (_, _, proto) = data.prototypes_in_category("entity").find(|(_, n, _)| *n == name)?;
    let open = sprite_from(data, main_layer(proto.get("hand_open_picture"))?, 0, 0)?;
    let base = sprite_from(data, main_layer(proto.get("hand_base_picture"))?, 0, 0)?;
    Some((open, base))
}

/// Character idle frame for one of 8 directions (0 = north, clockwise).
pub fn character_sprite(data: &GameData, name: &str, dir8: u32, running: bool, frame: u32) -> Option<SpriteRef> {
    let (_, _, proto) = data.prototypes_in_category("entity").find(|(_, n, _)| *n == name)?;
    let anims = proto.get("animations").at(0);
    let anim = if running { anims.get("running") } else { anims.get("idle") };
    let layer = main_layer(anim)?;
    let frames = layer.get("frame_count").as_i64().unwrap_or(1) as u32;
    let dirs = layer.get("direction_count").as_i64().unwrap_or(1) as u32;
    let row = dir8 * dirs / 8;
    if layer.get("filename").is_nil() {
        return None;
    }
    sprite_from(data, layer, frame % frames.max(1), row)
}

/// Resource sprite: column by depletion stage, row by variation.
pub fn resource_sprite(data: &GameData, name: &str, stage: u32, variation: u32) -> Option<SpriteRef> {
    let (_, _, proto) = data.prototypes_in_category("entity").find(|(_, n, _)| *n == name)?;
    let sheet = proto.get("stages").get("sheet");
    let variations = sheet.get("variation_count").as_i64().unwrap_or(1) as u32;
    let mut s = sprite_from(data, sheet, stage, variation % variations.max(1))?;
    // Stage sheets are laid out one stage per column without `line_length`.
    s.x = stage * s.width;
    Some(s)
}

/// First-row variants of a tile's main texture (1x1 tile size).
/// A tile's main pictures by block size: for each `size` (1, 2, 4...) its variants, each
/// covering size×size tiles. Large blocks are used where a whole aligned block is the
/// same tile, which hides the repetition of single-tile pictures (and gives water its
/// continuous look).
pub fn tile_variant_sets(data: &GameData, name: &str) -> Vec<(u32, Vec<SpriteRef>)> {
    let main = data.prototype("tile", name).get("variants").get("main");
    let mut out = Vec::new();
    for v in main.as_array() {
        let Some(path) = v.get("picture").as_str().and_then(|p| data.resolve_path(p)) else { continue };
        let size = v.get("size").as_i64().unwrap_or(1).max(1) as u32;
        let scale = v.get("scale").as_f64().unwrap_or(1.0);
        let px = (32.0 * size as f64 / scale).round() as u32;
        let count = v.get("count").as_i64().unwrap_or(1).max(1) as u32;
        let line = v.get("line_length").as_i64().unwrap_or(count as i64).max(1) as u32;
        let y0 = v.get("y").as_i64().unwrap_or(0) as u32;
        let sprites = (0..count)
            .map(|i| SpriteRef {
                path: path.clone(),
                x: (i % line) * px,
                y: y0 + (i / line) * px,
                width: px,
                height: px,
                scale,
                shift: (0.0, 0.0),
            })
            .collect();
        out.push((size, sprites));
    }
    out.sort_by_key(|(s, _)| *s);
    out
}

pub fn tile_variants(data: &GameData, name: &str) -> Vec<SpriteRef> {
    let proto = data.prototype("tile", name);
    let main = proto.get("variants").get("main");
    let Some(v) = main.as_array().iter().find(|v| v.get("size").as_i64() == Some(1)) else { return Vec::new() };
    let Some(path) = v.get("picture").as_str().and_then(|p| data.resolve_path(p)) else { return Vec::new() };
    let scale = v.get("scale").as_f64().unwrap_or(1.0);
    let px = (32.0 / scale).round() as u32;
    let count = v.get("count").as_i64().unwrap_or(1) as u32;
    let line = v.get("line_length").as_i64().unwrap_or(count as i64).max(1) as u32;
    let y0 = v.get("y").as_i64().unwrap_or(0) as u32;
    (0..count)
        .map(|i| SpriteRef {
            path: path.clone(),
            x: (i % line) * px,
            y: y0 + (i / line) * px,
            width: px,
            height: px,
            scale,
            shift: (0.0, 0.0),
        })
        .collect()
}

/// A cliff's picture for an orientation and variation: the upper part with its shadow,
/// and the lower part (`pictures_lower`, drawn beneath other entities), flagged `true`.
pub fn cliff_layers(
    data: &GameData,
    name: &str,
    orientation: &str,
    variation: usize,
) -> Vec<(SpriteRef, LayerKind, bool)> {
    let o = data.prototype("cliff", name).get("orientations").get(orientation);
    let mut out = Vec::new();
    for (key, lower) in [("pictures_lower", true), ("pictures", false)] {
        let list = o.get(key).as_array();
        let Some(pic) = list.get(variation % list.len().max(1)) else { continue };
        let mut nodes = Vec::new();
        collect_layers(pic, 0, &mut nodes);
        for (layer, _) in nodes {
            let kind =
                if layer.get("draw_as_shadow").as_bool() == Some(true) { LayerKind::Shadow } else { LayerKind::Normal };
            if let Some(s) = layer_frame(data, layer, 0, 0) {
                out.push((s, kind, lower));
            }
        }
    }
    out
}

/// One picture of a decorative (its `pictures[variation]`, first non-shadow layer).
pub fn decorative_sprite(data: &GameData, name: &str, variation: usize) -> Option<SpriteRef> {
    let pictures = data.prototype("optimized-decorative", name).get("pictures");
    let list = pictures.as_array();
    let pic = if list.is_empty() { pictures } else { &list[variation % list.len()] };
    let mut nodes = Vec::new();
    collect_layers(pic, 0, &mut nodes);
    let layer = nodes.iter().find(|(l, _)| l.get("draw_as_shadow").as_bool() != Some(true))?.0;
    layer_frame(data, layer, 0, 0)
}

/// One kind of transition piece in a tile's mask sheet: its x offset and variant count.
/// Each variant is a column of four rotations (north, east, south, west).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskPieces {
    pub x: u32,
    pub count: u32,
}

/// A tile's transition masks (`variants.transition`): greyscale masks through which the
/// tile is drawn over a neighbouring tile of a lower layer. White is this tile.
#[derive(Clone, Debug, PartialEq)]
pub struct TileTransition {
    pub sheet: std::path::PathBuf,
    /// Size of one piece in pixels.
    pub size: u32,
    pub y: u32,
    /// Two adjacent sides covered.
    pub inner_corner: MaskPieces,
    /// Only a diagonal neighbour.
    pub outer_corner: MaskPieces,
    pub side: MaskPieces,
    /// Three sides.
    pub u_transition: MaskPieces,
    /// Surrounded.
    pub o_transition: MaskPieces,
}

/// One piece type of a shore spritesheet: where its variants start, how many there are,
/// and how many tiles tall its overlay and background pictures are (masks are one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShorePieces {
    pub y: u32,
    pub count: u32,
    pub tile_height: u32,
}

/// A tile's transition to water (`transitions` with water in `to_tiles`): a mask through
/// which the land is drawn over the water cell, an overlay (the bank) drawn on top, and
/// optionally a background drawn under it. Overlay and background pieces may be two
/// tiles tall, reaching down into the next tile. Rows are the four rotations (N, E, S, W).
#[derive(Clone, Debug, PartialEq)]
pub struct Shore {
    pub sheet: std::path::PathBuf,
    /// Pixels per tile in the sheet.
    pub size: u32,
    pub to_tiles: Vec<String>,
    pub overlay_x: u32,
    pub mask_x: u32,
    pub background_x: Option<u32>,
    pub inner_corner: ShorePieces,
    pub outer_corner: ShorePieces,
    pub side: ShorePieces,
    pub u_transition: ShorePieces,
    pub o_transition: ShorePieces,
}

pub fn tile_shores(data: &GameData, name: &str) -> Vec<Shore> {
    let mut out = Vec::new();
    for t in data.prototype("tile", name).get("transitions").as_array() {
        let Some(sheet) = t.get("spritesheet").as_str().and_then(|p| data.resolve_path(p)) else { continue };
        let l = t.get("layout");
        let to_tiles: Vec<String> =
            t.get("to_tiles").as_array().iter().filter_map(|x| x.as_str().map(str::to_owned)).collect();
        if to_tiles.is_empty() {
            continue;
        }
        let int = |k: &str, d: i64| l.get(k).as_i64().unwrap_or(d).max(0) as u32;
        let height = int("tile_height", 1);
        let pieces = |p: &str, d_count: i64| ShorePieces {
            y: int(&format!("{p}_y"), 0),
            count: int(&format!("{p}_count"), int("count", d_count) as i64),
            tile_height: if p == "o_transition" { 1 } else { int(&format!("{p}_tile_height"), height as i64) },
        };
        out.push(Shore {
            sheet,
            size: (32.0 / l.get("scale").as_f64().unwrap_or(0.5)).round() as u32,
            to_tiles,
            overlay_x: l.get("overlay").get("x_offset").as_i64().unwrap_or(0).max(0) as u32,
            mask_x: l.get("mask").get("x_offset").as_i64().unwrap_or(0).max(0) as u32,
            background_x: (t.get("background_enabled").as_bool() != Some(false))
                .then(|| l.get("background").get("x_offset").as_i64())
                .flatten()
                .map(|x| x.max(0) as u32),
            inner_corner: pieces("inner_corner", 1),
            outer_corner: pieces("outer_corner", 1),
            side: pieces("side", 1),
            u_transition: pieces("u_transition", 1),
            o_transition: pieces("o_transition", 1),
        });
    }
    out
}

pub fn tile_transition(data: &GameData, name: &str) -> Option<TileTransition> {
    let t = data.prototype("tile", name).get("variants").get("transition");
    let sheet = data.resolve_path(t.get("spritesheet").as_str()?)?;
    let l = t.get("layout");
    let int = |k: &str, d: i64| l.get(k).as_i64().unwrap_or(d).max(0) as u32;
    let count = int("count", 1);
    let piece = |prefix: &str, default_x: i64, default_count: u32| MaskPieces {
        x: int(&format!("{prefix}_x"), default_x),
        count: l.get(&format!("{prefix}_count")).as_i64().map(|c| c.max(0) as u32).unwrap_or(default_count),
    };
    Some(TileTransition {
        sheet,
        size: (32.0 / l.get("scale").as_f64().unwrap_or(0.5)).round() as u32,
        y: l.get("mask").get("y_offset").as_i64().unwrap_or(0).max(0) as u32,
        inner_corner: piece("inner_corner", 0, count),
        outer_corner: piece("outer_corner", 0, count),
        side: piece("side", 0, count),
        u_transition: piece("u_transition", 0, 1),
        o_transition: piece("o_transition", 0, 1),
    })
}

/// Underground belt structure for the entrance (`input`) or exit, 4-way sheet.
pub fn underground_sprite(data: &GameData, name: &str, dir: usize, input: bool) -> Option<SpriteRef> {
    let (_, _, proto) = data.prototypes_in_category("entity").find(|(_, n, _)| *n == name)?;
    let key = if input { "direction_in" } else { "direction_out" };
    let layer = main_layer(proto.get("structure").get(key))?;
    sprite_from(data, layer, dir as u32, 0)
}

/// How a sprite layer is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayerKind {
    Normal,
    Shadow,
    /// Drawn multiplied by a colour (tree leaves).
    Tinted([u8; 4]),
}

/// The picture of a tree, rock or other scenery entity in one of its variations: a tree's
/// shadow, trunk and tinted leaves, or a rock's picture.
pub fn scenery_layers(data: &GameData, name: &str, variation: usize) -> Vec<(SpriteRef, LayerKind)> {
    let Some((kind, _, proto)) = data.prototypes_in_category("entity").find(|(_, n, _)| *n == name) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let add = |node: &RawValue, tint: Option<[u8; 4]>, out: &mut Vec<(SpriteRef, LayerKind)>| {
        let mut nodes = Vec::new();
        collect_layers(node, 0, &mut nodes);
        for (layer, _) in nodes {
            if layer.get("draw_as_light").as_bool() == Some(true) || layer.get("draw_as_glow").as_bool() == Some(true) {
                continue;
            }
            let kind = if layer.get("draw_as_shadow").as_bool() == Some(true) {
                LayerKind::Shadow
            } else {
                tint.map_or(LayerKind::Normal, LayerKind::Tinted)
            };
            if let Some(s) = layer_frame(data, layer, 0, 0) {
                out.push((s, kind));
            }
        }
    };
    if kind == "tree" {
        let variations = proto.get("variations").as_array();
        if let Some(v) = variations.get(variation % variations.len().max(1)) {
            let colors = proto.get("colors").as_array();
            let tint = colors.get(variation % colors.len().max(1)).map(|c| {
                let ch = |i: usize, k: &str| {
                    let x = c.at(i).as_f64().or(c.get(k).as_f64()).unwrap_or(1.0);
                    (if x > 1.0 { x } else { x * 255.0 }).round().clamp(0.0, 255.0) as u8
                };
                [ch(0, "r"), ch(1, "g"), ch(2, "b"), 255]
            });
            add(v.get("shadow"), None, &mut out);
            add(v.get("trunk"), None, &mut out);
            add(v.get("leaves"), tint, &mut out);
        }
    }
    // Rocks and dead trees: one of `pictures`.
    if out.is_empty() {
        let pictures = proto.get("pictures");
        let list = pictures.as_array();
        if list.is_empty() {
            add(if pictures.is_nil() { proto.get("picture") } else { pictures }, None, &mut out);
        } else {
            add(&list[variation % list.len()], None, &mut out);
        }
    }
    out
}

/// Picks the animation frame shown at `t` (in ticks) for a layer.
fn frame_at(layer: &RawValue, t: f64) -> u32 {
    let n = layer.get("frame_count").as_i64().unwrap_or(1).max(1) as u64;
    let speed = layer.get("animation_speed").as_f64().unwrap_or(1.0);
    let f = (t * speed).floor().max(0.0) as u64;
    let frame = match layer.get("run_mode").as_str() {
        Some("forward-then-backward") if n > 1 => {
            let period = 2 * n - 2;
            let k = f % period;
            if k < n { k } else { period - k }
        }
        Some("backward") => n - 1 - f % n,
        _ => f % n,
    };
    frame as u32
}

/// A frame of one layer, handling single files, multi-file `filenames` and direction rows.
fn layer_frame(data: &GameData, layer: &RawValue, frame: u32, row: u32) -> Option<SpriteRef> {
    if let Some(files) = layer.get("filenames").as_array().first().map(|_| layer.get("filenames").as_array()) {
        let size = layer.get("size");
        let w = layer.get("width").as_i64().or(size.as_i64()).or(size.at(0).as_i64())? as u32;
        let h = layer.get("height").as_i64().or(size.as_i64()).or(size.at(1).as_i64())? as u32;
        let line = layer.get("line_length").as_i64().unwrap_or(1).max(1) as u32;
        let lines = layer.get("lines_per_file").as_i64().unwrap_or(1).max(1) as u32;
        let per_file = line * lines;
        let file = files.get((frame / per_file) as usize)?.as_str()?;
        let local = frame % per_file;
        return Some(SpriteRef {
            path: data.resolve_path(file)?,
            x: (local % line) * w,
            y: (local / line) * h,
            width: w,
            height: h,
            scale: layer.get("scale").as_f64().unwrap_or(1.0),
            shift: vector(layer.get("shift")),
        });
    }
    sprite_from(data, layer, frame, row)
}

/// Flattens a sprite/animation node into its layers (descending into `layers`, `sheet`,
/// `sheets`), with the frame and row to use for a cardinal direction.
fn collect_layers<'a>(v: &'a RawValue, dir: usize, out: &mut Vec<(&'a RawValue, bool)>) {
    if v.is_nil() {
        return;
    }
    if !v.get(DIR_KEYS[0]).is_nil() {
        return collect_layers(v.get(DIR_KEYS[dir]), dir, out);
    }
    if !v.get("layers").as_array().is_empty() {
        for l in v.get("layers").as_array() {
            collect_layers(l, dir, out);
        }
        return;
    }
    if !v.get("sheet").is_nil() {
        out.push((v.get("sheet"), true));
        return;
    }
    if let Some(s) = v.get("sheets").as_array().first() {
        let _ = s;
        for s in v.get("sheets").as_array() {
            out.push((s, true));
        }
        return;
    }
    if !v.get("filename").is_nil() || !v.get("filenames").is_nil() {
        out.push((v, false));
    }
}

/// All layers of an entity's main graphics for a direction at tick `t`, plus its working
/// visualisations while `working`. Glow/light layers are skipped.
pub fn entity_layers(data: &GameData, name: &str, dir: usize, t: u64, working: bool) -> Vec<(SpriteRef, LayerKind)> {
    let Some((kind, _, proto)) = data.prototypes_in_category("entity").find(|(_, n, _)| *n == name) else {
        return Vec::new();
    };
    let gs = proto.get("graphics_set");
    let root = if kind == "generator" {
        proto.get(if dir % 2 == 1 { "horizontal_animation" } else { "vertical_animation" })
    } else if kind == "lab" {
        proto.get(if working { "on_animation" } else { "off_animation" })
    } else {
        [
            gs.get("animation"),
            gs.get("idle_animation"),
            proto.get("picture"),
            proto.get("pictures"),
            proto.get("animation"),
            proto.get("structure"),
            proto.get("platform_picture"),
        ]
        .into_iter()
        .find(|v| !v.is_nil())
        .unwrap_or(&RawValue::Nil)
    };
    let mut nodes = Vec::new();
    collect_layers(root, dir, &mut nodes);
    let mut extra = Vec::new();
    for vis in gs.get("working_visualisations").as_array().iter().chain(proto.get("working_visualisations").as_array())
    {
        let always = vis.get("always_draw").as_bool() == Some(true);
        if !(working || always) {
            continue;
        }
        let key = format!("{}_animation", DIR_KEYS[dir]);
        let anim = if vis.get(&key).is_nil() { vis.get("animation") } else { vis.get(&key) };
        collect_layers(anim, dir, &mut extra);
    }
    nodes.extend(extra);

    let tf = t as f64;
    let mut out = Vec::new();
    for (layer, four_way) in nodes {
        if layer.get("draw_as_light").as_bool() == Some(true) || layer.get("draw_as_glow").as_bool() == Some(true) {
            continue;
        }
        let direction_count = layer.get("direction_count").as_i64().unwrap_or(1);
        let animated = layer.get("frame_count").as_i64().unwrap_or(1) > 1;
        let (frame, row) = if four_way {
            (dir as u32, 0)
        } else if direction_count >= 4 && !animated && layer.get("frame_count").is_nil() {
            // A rotated sprite (no animation): the directions follow each other along the
            // sheet's lines like frames (electric poles' four pictures).
            ((dir as i64 * direction_count / 4) as u32, 0)
        } else {
            let frame = if animated && working { frame_at(layer, tf) } else { 0 };
            let row = if direction_count >= 4 { (dir as i64 * direction_count / 4) as u32 } else { 0 };
            (frame, row)
        };
        // Animations with a direction_count lay out each direction as its own row block.
        let rotated_sprite = direction_count >= 4 && layer.get("frame_count").is_nil();
        let frames_per_row_block = if direction_count >= 4 && !four_way && !rotated_sprite {
            let n = layer.get("frame_count").as_i64().unwrap_or(1).max(1) as u32;
            let line = layer.get("line_length").as_i64().map(|l| l as u32).unwrap_or(n).max(1);
            n.div_ceil(line)
        } else {
            1
        };
        let kind =
            if layer.get("draw_as_shadow").as_bool() == Some(true) { LayerKind::Shadow } else { LayerKind::Normal };
        if let Some(s) = layer_frame(data, layer, frame, row * frames_per_row_block) {
            out.push((s, kind));
        }
    }
    out
}

/// A sprite from `data.raw["utility-sprites"].default` (GUI icons such as `close` or
/// `status_working`): the full-size picture (mipmapped icons keep their mips beside it).
pub fn utility_sprite(data: &GameData, name: &str) -> Option<SpriteRef> {
    let s = data.prototype("utility-sprites", "default").get(name);
    let s = if s.get("layers").is_nil() { s } else { s.get("layers").at(0) };
    let size = s.get("size");
    let (w, h) = match (size.as_i64(), size.at(0).as_i64(), size.at(1).as_i64()) {
        (Some(n), _, _) => (n, n),
        (_, Some(w), Some(h)) => (w, h),
        _ => (s.get("width").as_i64()?, s.get("height").as_i64()?),
    };
    Some(SpriteRef {
        path: data.resolve_path(s.get("filename").as_str()?)?,
        x: s.get("x").as_i64().unwrap_or(0) as u32,
        y: s.get("y").as_i64().unwrap_or(0) as u32,
        width: w as u32,
        height: h as u32,
        scale: s.get("scale").as_f64().unwrap_or(1.0),
        shift: (0.0, 0.0),
    })
}
