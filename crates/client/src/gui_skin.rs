//! The game's GUI skin, read from `data.raw["gui-style"].default`: each style's graphical
//! sets are 9-slice regions of the GUI tileset (`__core__/graphics/gui-new.png` by default),
//! drawn at the tileset's scale (0.5, so the 2x art shows at 1x UI scale). Styles inherit
//! from their `parent`.

use bevy::prelude::*;
use bevy::ui::widget::NodeImageMode;
use factorio_data::RawValue;

use crate::Data;

pub struct GuiSkinPlugin;

impl Plugin for GuiSkinPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, (load, resolve).chain()).add_systems(Update, button_states);
    }
}

/// One drawable part of a graphical set: a region of a sheet sliced 9 ways.
#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub image: Handle<Image>,
    pub rect: Rect,
    /// Corner size in sheet pixels.
    pub border: f32,
    /// Drawn around the widget's edge instead of inside it.
    pub outer: bool,
    /// A separate centre region (frames with a different fill than their border).
    pub center: Option<Rect>,
    pub tint: Color,
}

#[derive(Resource)]
pub struct Skin {
    style: RawValue,
    tileset: String,
    scale: f32,
}

fn load(mut commands: Commands, data: Res<Data>) {
    let style = data.0.prototype("gui-style", "default").clone();
    let tileset = style.get("default_tileset").as_str().unwrap_or("__core__/graphics/gui-new.png").to_owned();
    let scale = style.get("default_sprite_scale").as_f64().unwrap_or(0.5) as f32;
    commands.insert_resource(Skin { style, tileset, scale });
}

fn pair(v: &RawValue) -> Option<Vec2> {
    if let Some(n) = v.as_f64() {
        return Some(Vec2::splat(n as f32));
    }
    Some(Vec2::new(v.at(0).as_f64()? as f32, v.at(1).as_f64()? as f32))
}

impl Skin {
    /// A style property, looked up through the style's parents.
    pub fn prop(&self, style: &str, key: &str) -> Option<&RawValue> {
        let mut name = style.to_owned();
        for _ in 0..16 {
            let s = self.style.get(&name);
            if s.is_nil() {
                return None;
            }
            let v = s.get(key);
            if !v.is_nil() {
                return Some(v);
            }
            name = s.get("parent").as_str()?.to_owned();
        }
        None
    }

    /// A colour property (`{r, g, b, a}` or an array; 0-255 or 0-1).
    pub fn color(&self, style: &str, key: &str) -> Option<Color> {
        let c = self.prop(style, key)?;
        let ch = |i: usize, k: &str| c.at(i).as_f64().or(c.get(k).as_f64());
        // Missing channels are 0 (`{r = 1}` is red).
        let (r, g, b) = (ch(0, "r"), ch(1, "g"), ch(2, "b"));
        if r.is_none() && g.is_none() && b.is_none() {
            return None;
        }
        let (r, g, b) = (r.unwrap_or(0.0), g.unwrap_or(0.0), b.unwrap_or(0.0));
        let a = ch(3, "a").unwrap_or(if r > 1.0 || g > 1.0 || b > 1.0 { 255.0 } else { 1.0 });
        let big = r > 1.0 || g > 1.0 || b > 1.0 || a > 1.0;
        let k = if big { 255.0 } else { 1.0 };
        Some(Color::srgba(
            (r / k) as f32,
            (g / k) as f32,
            (b / k) as f32,
            (a / if a > 1.0 { 255.0 } else { 1.0 }) as f32,
        ))
    }

    /// The `base` part of a graphical set property such as `default_graphical_set`.
    pub fn slice(&self, style: &str, key: &str, data: &Data, assets: &AssetServer) -> Option<Slice> {
        let set = self.prop(style, key)?;
        let base = if set.get("base").is_nil() { set } else { set.get("base") };
        self.element(base, data, assets)
    }

    pub fn element(&self, e: &RawValue, data: &Data, assets: &AssetServer) -> Option<Slice> {
        let file = e.get("filename").as_str().unwrap_or(&self.tileset);
        let path = data.0.resolve_path(file)?;
        let image: Handle<Image> = assets.load(crate::sprites::asset_path(data, &path));
        let tint = {
            let t = e.get("tint");
            match (t.at(0).as_f64().or(t.get("r").as_f64()), t.at(1).as_f64().or(t.get("g").as_f64())) {
                (Some(r), Some(g)) => {
                    let b = t.at(2).as_f64().or(t.get("b").as_f64()).unwrap_or(0.0);
                    let a = t.at(3).as_f64().or(t.get("a").as_f64()).unwrap_or(255.0);
                    let k = if r > 1.0 || g > 1.0 || b > 1.0 { 255.0 } else { 1.0 };
                    Color::srgba(
                        (r / k) as f32,
                        (g / k) as f32,
                        (b / k) as f32,
                        (if a > 1.0 { a / 255.0 } else { a }) as f32,
                    )
                }
                _ => Color::WHITE,
            }
        };
        let outer = e.get("draw_type").as_str() == Some("outer");
        let center = pair(e.get("center").get("position"))
            .map(|p| Rect::from_corners(p, p + pair(e.get("center").get("size")).unwrap_or(Vec2::ONE)));
        // `position` with `corner_size` (a (2c+1)² block), or with `size` and `border`.
        if let Some(pos) = pair(e.get("position")) {
            if let Some(c) = e.get("corner_size").as_f64() {
                let c = c as f32;
                return Some(Slice {
                    image,
                    rect: Rect::from_corners(pos, pos + Vec2::splat(2.0 * c + 1.0)),
                    border: c,
                    outer,
                    center,
                    tint,
                });
            }
            let size = pair(e.get("size")).unwrap_or(Vec2::splat(1.0));
            let border = e.get("border").as_f64().unwrap_or(0.0) as f32;
            return Some(Slice { image, rect: Rect::from_corners(pos, pos + size), border, outer, center, tint });
        }
        // Explicit parts: the box from the top-left corner part to the bottom-right one.
        let lt = e.get("left_top");
        let rb = e.get("right_bottom");
        let (p0, s0) = (pair(lt.get("position"))?, pair(lt.get("size")).unwrap_or(Vec2::splat(8.0)));
        let (p1, s1) = (pair(rb.get("position"))?, pair(rb.get("size")).unwrap_or(Vec2::splat(8.0)));
        Some(Slice { image, rect: Rect::from_corners(p0, p1 + s1), border: s0.x.max(1.0), outer, center, tint })
    }

    /// The default, hovered and clicked looks of a button-like style.
    pub fn states(&self, style: &str, data: &Data, assets: &AssetServer) -> Option<ButtonLook> {
        let default = self.slice(style, "default_graphical_set", data, assets)?;
        Some(ButtonLook {
            hovered: self.slice(style, "hovered_graphical_set", data, assets).unwrap_or_else(|| default.clone()),
            clicked: self.slice(style, "clicked_graphical_set", data, assets).unwrap_or_else(|| default.clone()),
            default,
        })
    }
}

/// A button's looks by state; [`button_states`] swaps them on hover and press.
#[derive(Component, Clone, Debug)]
pub struct ButtonLook {
    pub default: Slice,
    pub hovered: Slice,
    pub clicked: Slice,
}

fn button_states(mut q: Query<(&Interaction, &ButtonLook, &mut ImageNode), Changed<Interaction>>) {
    for (i, look, mut node) in &mut q {
        let s = match i {
            Interaction::Pressed => &look.clicked,
            Interaction::Hovered => &look.hovered,
            Interaction::None => &look.default,
        };
        node.rect = Some(s.rect);
        if let NodeImageMode::Sliced(slicer) = &mut node.image_mode {
            slicer.border = BorderRect::all(s.border);
        }
    }
}

/// The looks of the widgets the GUI uses, resolved once at startup.
#[allow(dead_code)] // Some are for the window layouts still being rebuilt.
pub struct Looks {
    pub frame: Slice,
    pub deep: Slice,
    /// `deep_frame_in_shallow_frame`, e.g. around the entity preview.
    pub deep_in_shallow: Slice,
    /// The `line` style's horizontal line piece, tiled.
    pub line: Option<(Handle<Image>, Rect)>,
    /// `production_progressbar`'s bar colour.
    pub production_bar_color: Color,
    /// One empty cell of `deep_slots_scroll_pane`'s tiled background (32 px, inset 4).
    pub empty_slot: Option<Slice>,
    /// `checkbox`: the box (unchecked, checked) and the check mark.
    pub checkbox: ButtonLook,
    pub checkbox_checked: Slice,
    pub checkmark: Slice,
    /// `slider`: the filled part, the empty track and the handle.
    pub slider_full: Slice,
    pub slider_empty: Slice,
    pub slider_handle: Slice,
    /// `textbox`'s background.
    pub textbox: Slice,
    /// `dropdown_button`.
    pub dropdown: ButtonLook,
    /// The `dialog_button` arrow pictures (left half: back, right half: confirm) and the
    /// stretchable middle, grey and green: rects in the tileset.
    pub dialog_grey: Rect,
    pub dialog_green: Rect,
    pub tileset: Handle<Image>,
    /// `side_menu_button`, the top-right menu buttons.
    pub side_menu_button: ButtonLook,
    /// `shortcut_bar_button` and its blue, red and green variants.
    pub shortcut_buttons: [ButtonLook; 4],
    /// Technology slots by state: available, conditionally available, unavailable,
    /// researched.
    pub tech_slots: [ButtonLook; 4],
    /// `technology_card_frame`, around the selected technology's details.
    pub tech_card: Slice,
    /// `entity_frame_filler`'s row picture.
    pub entity_filler: Option<Slice>,
    /// `burning_progressbar`'s (fuel left).
    pub burning_bar_color: Color,
    pub shallow: Slice,
    pub slot: ButtonLook,
    pub inventory_slot: ButtonLook,
    pub red_slot: ButtonLook,
    pub yellow_slot: ButtonLook,
    pub tab: ButtonLook,
    pub tab_selected: Slice,
    pub bar_background: Slice,
    pub bar: Slice,
    pub bar_color: Color,
    pub tooltip: Slice,
    /// `tooltip_title_frame_light`, the title band.
    pub tooltip_title: Slice,
    pub button: ButtonLook,
    pub red_button: ButtonLook,
    pub frame_button: ButtonLook,
    /// The striped draggable header filler, tiled.
    pub header_filler: Option<(Handle<Image>, Rect)>,
    pub title_color: Color,
    pub scale: f32,
}

static LOOKS: std::sync::OnceLock<Looks> = std::sync::OnceLock::new();

/// The GUI looks (available after startup).
pub fn looks() -> &'static Looks {
    LOOKS.get().expect("GUI skin not loaded")
}

fn resolve(mut commands: Commands, skin: Res<Skin>, data: Res<Data>, assets: Res<AssetServer>) {
    let get = |style: &str, key: &str| skin.slice(style, key, &data, &assets);
    let states = |style: &str| skin.states(style, &data, &assets);
    let fallback = || Slice {
        image: Handle::default(),
        rect: Rect::new(0.0, 0.0, 1.0, 1.0),
        border: 0.0,
        outer: false,
        center: None,
        tint: Color::srgb(0.2, 0.2, 0.2),
    };
    let look = |s: Option<ButtonLook>| {
        s.unwrap_or_else(|| ButtonLook { default: fallback(), hovered: fallback(), clicked: fallback() })
    };
    let header_filler = skin.prop("draggable_space", "graphical_set").and_then(|g| {
        let c = g.get("base").get("center");
        let p = pair(c.get("position"))?;
        let size = pair(c.get("size")).unwrap_or(Vec2::splat(8.0));
        let path = data.0.resolve_path(&skin.tileset)?;
        Some((assets.load(crate::sprites::asset_path(&data, &path)), Rect::from_corners(p, p + size)))
    });
    let line = skin.prop("line", "border").and_then(|b| {
        let h = b.get("horizontal_line");
        let p = pair(h.get("position"))?;
        let size = pair(h.get("size")).unwrap_or(Vec2::new(1.0, 8.0));
        let path = data.0.resolve_path(&skin.tileset)?;
        Some((assets.load(crate::sprites::asset_path(&data, &path)), Rect::from_corners(p, p + size)))
    });
    let looks = Looks {
        frame: get("frame", "graphical_set").unwrap_or_else(fallback),
        deep_in_shallow: get("deep_frame_in_shallow_frame", "graphical_set").unwrap_or_else(fallback),
        line,
        production_bar_color: skin.color("production_progressbar", "color").unwrap_or(Color::srgb_u8(43, 227, 39)),
        empty_slot: skin
            .prop("deep_slots_scroll_pane", "background_graphical_set")
            .and_then(|g| skin.element(g, &data, &assets)),
        checkbox: look(states("checkbox")),
        checkbox_checked: get("checkbox", "selected_graphical_set").unwrap_or_else(fallback),
        checkmark: skin
            .prop("checkbox", "checkmark")
            .and_then(|e| skin.element(e, &data, &assets))
            .unwrap_or_else(fallback),
        slider_full: skin
            .prop("slider", "full_bar")
            .and_then(|e| skin.element(e.get("base"), &data, &assets))
            .unwrap_or_else(fallback),
        slider_empty: skin
            .prop("slider", "empty_bar")
            .and_then(|e| skin.element(e.get("base").get("center"), &data, &assets))
            .unwrap_or_else(fallback),
        slider_handle: skin
            .prop("slider", "button")
            .and_then(|e| skin.element(e.get("default_graphical_set").get("base"), &data, &assets))
            .unwrap_or_else(fallback),
        textbox: skin
            .prop("textbox", "default_background")
            .and_then(|e| skin.element(e.get("base"), &data, &assets))
            .unwrap_or_else(fallback),
        dropdown: look(states("dropdown_button")),
        dialog_grey: Rect::new(0.0, 232.0, 48.0, 296.0),
        dialog_green: Rect::new(0.0, 296.0, 48.0, 360.0),
        tileset: data
            .0
            .resolve_path(&skin.tileset)
            .map(|p| assets.load(crate::sprites::asset_path(&data, &p)))
            .unwrap_or_default(),
        side_menu_button: look(states("side_menu_button")),
        shortcut_buttons: [
            look(states("shortcut_bar_button")),
            look(states("shortcut_bar_button_blue")),
            look(states("shortcut_bar_button_red")),
            look(states("shortcut_bar_button_green")),
        ],
        tech_slots: [
            look(states("available_technology_slot")),
            look(states("conditionally_available_technology_slot")),
            look(states("unavailable_technology_slot")),
            look(states("researched_technology_slot")),
        ],
        tech_card: get("technology_card_frame", "graphical_set").unwrap_or_else(fallback),
        entity_filler: skin.prop("entity_frame_filler", "graphical_set").and_then(|g| skin.element(g, &data, &assets)),
        burning_bar_color: skin.color("burning_progressbar", "color").unwrap_or(Color::srgb(1.0, 0.0, 0.0)),
        deep: get("inside_deep_frame", "graphical_set").unwrap_or_else(fallback),
        shallow: get("inside_shallow_frame", "graphical_set").unwrap_or_else(fallback),
        slot: look(states("slot_button")),
        inventory_slot: look(states("inventory_slot")),
        red_slot: look(states("red_slot_button")),
        yellow_slot: look(states("yellow_slot_button")),
        tab: look(states("filter_group_tab")),
        tab_selected: get("filter_group_tab", "selected_graphical_set").unwrap_or_else(fallback),
        bar_background: get("progressbar", "bar_background").unwrap_or_else(fallback),
        bar: skin.prop("progressbar", "bar").and_then(|b| skin.element(b, &data, &assets)).unwrap_or_else(fallback),
        bar_color: skin.color("progressbar", "color").unwrap_or(Color::srgb(0.98, 0.66, 0.22)),
        tooltip: get("tooltip_frame", "graphical_set").unwrap_or_else(fallback),
        tooltip_title: get("tooltip_title_frame_light", "graphical_set").unwrap_or_else(fallback),
        button: look(states("button")),
        red_button: look(states("tool_button_red")),
        frame_button: look(states("frame_action_button")),
        header_filler,
        title_color: skin.color("frame_title", "font_color").unwrap_or(Color::srgb(1.0, 0.9, 0.75)),
        scale: skin.scale,
    };
    let _ = LOOKS.set(looks);
    commands.insert_resource(LooksReady);
}

/// Marks that [`looks`] can be used.
#[derive(Resource)]
pub struct LooksReady;

/// A UI image node drawing a slice (9-sliced at the skin's scale).
pub fn node_image(s: &Slice) -> ImageNode {
    ImageNode {
        image: s.image.clone(),
        rect: Some(s.rect),
        color: s.tint,
        image_mode: NodeImageMode::Sliced(TextureSlicer {
            border: BorderRect::all(s.border),
            max_corner_scale: looks().scale,
            ..default()
        }),
        ..default()
    }
}

/// Spawns a slice as a backdrop behind `parent`'s children (see [`Skin::backdrop`]).
pub fn backdrop(parent: &mut ChildSpawnerCommands, s: &Slice) {
    let scale = looks().scale;
    let grow = if s.outer { s.border * scale } else { 0.0 };
    parent.spawn((
        node_image(s),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(-grow),
            top: Val::Px(-grow),
            right: Val::Px(-grow),
            bottom: Val::Px(-grow),
            ..default()
        },
        ZIndex(-1),
        Pickable::IGNORE,
    ));
    if let Some(c) = s.center {
        let inset = if s.outer { 0.0 } else { s.border * scale };
        parent.spawn((
            ImageNode { image: s.image.clone(), rect: Some(c), color: s.tint, ..default() },
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(inset),
                top: Val::Px(inset),
                right: Val::Px(inset),
                bottom: Val::Px(inset),
                ..default()
            },
            ZIndex(-1),
            Pickable::IGNORE,
        ));
    }
}
