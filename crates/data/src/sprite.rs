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
