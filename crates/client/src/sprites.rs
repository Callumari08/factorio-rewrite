//! Caches sprite lookups and image handles for game graphics.

use std::collections::HashMap;

use bevy::prelude::*;
use factorio_data::sprite::SpriteRef;

use crate::Data;

pub struct SpritesPlugin;

impl Plugin for SpritesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Sprites>();
    }
}

#[derive(Clone)]
pub struct Loaded {
    pub image: Handle<Image>,
    pub sprite: SpriteRef,
}

impl Loaded {
    /// A Bevy sprite drawing this region at Factorio scale (32 px per tile at scale 1).
    pub fn sprite(&self) -> Sprite {
        let s = &self.sprite;
        let (x, y, w, h) = (s.x as f32, s.y as f32, s.width as f32, s.height as f32);
        Sprite {
            image: self.image.clone(),
            rect: Some(inset(x, y, w, h)),
            custom_size: Some(Vec2::new(w, h) * s.scale as f32),
            ..default()
        }
    }

    pub fn shift(&self) -> Vec2 {
        Vec2::new(self.sprite.shift.0 as f32, -self.sprite.shift.1 as f32) * crate::TILE
    }

    pub fn apply(&self, sprite: &mut Sprite) {
        let s = &self.sprite;
        let (x, y, w, h) = (s.x as f32, s.y as f32, s.width as f32, s.height as f32);
        if sprite.image != self.image {
            sprite.image = self.image.clone();
        }
        sprite.rect = Some(inset(x, y, w, h));
        sprite.custom_size = Some(Vec2::new(w, h) * s.scale as f32);
    }
}

/// A sprite's rectangle in its sheet, pulled in by half a texel so filtering never picks
/// up the neighbouring picture (which shows as thin lines along sprite edges).
fn inset(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(x + 0.5, y + 0.5, x + w - 0.5, y + h - 0.5)
}

#[derive(Resource, Default)]
pub struct Sprites {
    cache: HashMap<String, Option<Loaded>>,
}

impl Sprites {
    /// Looks up (once) and returns the sprite for `key`, computed by `f`.
    pub fn get(
        &mut self,
        assets: &AssetServer,
        data: &Data,
        key: &str,
        f: impl FnOnce() -> Option<SpriteRef>,
    ) -> Option<Loaded> {
        if let Some(v) = self.cache.get(key) {
            return v.clone();
        }
        let v = f().map(|sprite| Loaded { image: assets.load(asset_path(data, &sprite.path)), sprite });
        self.cache.insert(key.to_owned(), v.clone());
        v
    }

    pub fn item_icon(&mut self, assets: &AssetServer, data: &Data, item: &str) -> Option<Loaded> {
        let d = data.0.clone();
        self.get(assets, data, &format!("icon:{item}"), || factorio_data::sprite::item_icon(&d, item))
    }
}

pub fn asset_path(data: &Data, path: &std::path::Path) -> String {
    let rel = path.strip_prefix(&data.0.install.root).unwrap_or(path);
    format!("factorio://{}", rel.to_string_lossy().replace('\\', "/"))
}
