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

/// Underground belt structure for the entrance (`input`) or exit, 4-way sheet.
pub fn underground_sprite(data: &GameData, name: &str, dir: usize, input: bool) -> Option<SpriteRef> {
    let (_, _, proto) = data.prototypes_in_category("entity").find(|(_, n, _)| *n == name)?;
    let key = if input { "direction_in" } else { "direction_out" };
    let layer = main_layer(proto.get("structure").get(key))?;
    sprite_from(data, layer, dir as u32, 0)
}
