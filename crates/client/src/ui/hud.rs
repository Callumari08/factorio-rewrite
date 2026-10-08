//! The HUD around the game view, laid out as in the game: the side menu at the top right
//! (current research, menu buttons, minimap), the character panel at the bottom left
//! (portrait, guns, ammo, armour) and the shortcut bar beside the quickbar.

use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use factorio_sim::map::{MapPosition, TilePosition};
use factorio_sim::proto::EntityProtoId;

use super::research::ResearchHudRoot;
use super::*;

/// Width of the character panel; the crafting queue starts right of it.
pub(super) const CHARACTER_PANEL_W: f32 = SLOT_PX * 4.0 + 16.0;
/// The minimap: pixels on screen, one tile per pixel.
const MINIMAP_PX: u32 = 248;
/// The side menu's width.
const SIDE_MENU_W: f32 = MINIMAP_PX as f32 + 16.0;

/// Every HUD panel (hidden under the technology screen).
#[derive(Component)]
pub(super) struct HudRoot;
#[derive(Component)]
pub(super) struct SideMenuButtons;
#[derive(Component)]
pub(super) struct CharacterPanel;
#[derive(Component)]
pub(super) struct ShortcutBar;

#[derive(Resource)]
pub(super) struct Minimap {
    image: Handle<Image>,
    /// Map colours by entity prototype (None: not drawn).
    colors: HashMap<EntityProtoId, Option<[u8; 4]>>,
}

pub(super) fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = images.add(Image::new(
        Extent3d { width: MINIMAP_PX, height: MINIMAP_PX, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0; (MINIMAP_PX * MINIMAP_PX * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    ));
    commands.insert_resource(Minimap { image: image.clone(), colors: HashMap::new() });
    let l = looks();
    // Side menu, top right.
    commands
        .spawn((
            HudRoot,
            Interaction::default(),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                right: Val::Px(0.0),
                width: Val::Px(SIDE_MENU_W),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(4.0)),
                ..default()
            },
            crate::gui_skin::node_image(&l.frame),
        ))
        .with_children(|m| {
            m.spawn((
                ResearchHudRoot,
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(8.0),
                    align_items: AlignItems::Center,
                    padding: UiRect::all(Val::Px(6.0)),
                    min_height: Val::Px(48.0),
                    ..default()
                },
                BackgroundColor(INNER),
            ));
            m.spawn((SideMenuButtons, Node { flex_direction: FlexDirection::Column, ..default() }));
            m.spawn((
                ImageNode::new(image),
                Node { width: Val::Px(MINIMAP_PX as f32), height: Val::Px(MINIMAP_PX as f32), ..default() },
            ));
        });
    // Character panel, bottom left.
    commands.spawn((
        HudRoot,
        CharacterPanel,
        Interaction::default(),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(0.0),
            left: Val::Px(0.0),
            width: Val::Px(CHARACTER_PANEL_W),
            padding: UiRect::all(Val::Px(4.0)),
            flex_direction: FlexDirection::Column,
            ..default()
        },
        crate::gui_skin::node_image(&l.frame),
    ));
    // Shortcut bar, right of the quickbar.
    commands.spawn((
        HudRoot,
        ShortcutBar,
        Interaction::default(),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(0.0),
            left: Val::Percent(50.0),
            margin: UiRect::left(Val::Px(QUICKBAR_W / 2.0 + 4.0)),
            flex_direction: FlexDirection::Row,
            padding: UiRect::all(Val::Px(4.0)),
            ..default()
        },
        crate::gui_skin::node_image(&l.frame),
    ));
}

/// A square HUD button showing a `utility-sprites` picture.
fn menu_button(p: &mut ChildSpawnerCommands, ctx: &mut Ctx, sprite: &str, tip: &str, enabled: bool) {
    let l = looks();
    let mut e = p.spawn((
        Node {
            width: Val::Px(SLOT_PX),
            height: Val::Px(SLOT_PX),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        crate::gui_skin::node_image(&l.side_menu_button.default),
        Tip::Text(tip.into()),
        Interaction::default(),
    ));
    if enabled {
        e.insert(l.side_menu_button.clone());
    } else {
        e.insert(ImageNode {
            color: Color::srgba(1.0, 1.0, 1.0, 0.5),
            ..crate::gui_skin::node_image(&l.side_menu_button.default)
        });
    }
    e.with_children(|b| ctx.utility(b, sprite, 32.0));
}

/// The two rows of side menu buttons, as in the game; the technology button shows how
/// many technologies are queued.
pub(super) fn side_menu(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    fonts: Res<Fonts>,
    mut sprites: ResMut<Sprites>,
    root: Single<Entity, With<SideMenuButtons>>,
    mut last: Local<Option<usize>>,
) {
    let r = sim.0.research();
    if *last == Some(r.queue.len()) {
        return;
    }
    *last = Some(r.queue.len());
    let db = sim.0.prototypes();
    let mut ctx =
        Ctx { sprites: &mut sprites, assets: &assets, data: &data, fonts: &fonts, db, research: r, hovered: None };
    commands.entity(*root).despawn_related::<Children>();
    commands.entity(*root).with_children(|m| {
        m.spawn(Node { flex_direction: FlexDirection::Row, ..default() }).with_children(|row| {
            for (sprite, tip) in [
                ("side_menu_map_icon", "Map (M)"),
                ("side_menu_production_icon", "Production statistics (P)"),
                ("side_menu_bonus_icon", "Bonuses"),
                ("side_menu_factoriopedia_icon", "Factoriopedia (Alt+F)"),
                ("side_menu_train_icon", "Trains (Shift+T)"),
                ("side_menu_achievements_icon", "Achievements"),
            ] {
                menu_button(row, &mut ctx, sprite, tip, true);
            }
        });
        m.spawn(Node { flex_direction: FlexDirection::Row, ..default() }).with_children(|row| {
            row.spawn((
                Node {
                    width: Val::Px(SLOT_PX),
                    height: Val::Px(SLOT_PX),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                crate::gui_skin::node_image(&looks().side_menu_button.default),
                looks().side_menu_button.clone(),
                UiButton::OpenTech,
                Button,
                Tip::Text("Technologies (T)".into()),
            ))
            .with_children(|b| {
                ctx.utility(b, "side_menu_technology_icon", 32.0);
                if !r.queue.is_empty() {
                    b.spawn((
                        Text::new(r.queue.len().to_string()),
                        TextFont { font: ctx.fonts.bold.clone(), font_size: 13.0, ..default() },
                        TextColor(Color::WHITE),
                        TextShadow { offset: Vec2::new(1.0, 1.0), color: Color::BLACK },
                        Node {
                            position_type: PositionType::Absolute,
                            right: Val::Px(3.0),
                            bottom: Val::Px(0.0),
                            ..default()
                        },
                    ));
                }
            });
            menu_button(row, &mut ctx, "side_menu_logistic_networks_icon", "Logistic networks (L)", false);
            for _ in 0..4 {
                empty_cell(row);
            }
        });
    });
}

/// The bottom-left panel: the character's portrait and its gun, ammo and armour slots.
/// Guns and armour are not simulated yet, so the slots show their placeholders.
pub(super) fn character_panel(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    fonts: Res<Fonts>,
    mut sprites: ResMut<Sprites>,
    root: Single<Entity, With<CharacterPanel>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    *done = true;
    let db = sim.0.prototypes();
    let mut ctx = Ctx {
        sprites: &mut sprites,
        assets: &assets,
        data: &data,
        fonts: &fonts,
        db,
        research: sim.0.research(),
        hovered: None,
    };
    commands.entity(*root).with_children(|p| {
        for (first, rest) in [("character", "empty_gun_slot"), ("armor", "empty_ammo_slot")] {
            p.spawn(Node { flex_direction: FlexDirection::Row, ..default() }).with_children(|row| {
                if first == "character" {
                    // The portrait: the character's own icon.
                    let d = ctx.data.0.clone();
                    let icon = ctx.sprites.get(ctx.assets, ctx.data, "entity-icon:character", || {
                        factorio_data::sprite::icon_of(&d, d.prototype("character", "character"))
                    });
                    row.spawn((
                        Node {
                            width: Val::Px(SLOT_PX),
                            height: Val::Px(SLOT_PX),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        crate::gui_skin::node_image(&looks().slot.default),
                        Tip::Text("Character".into()),
                        Interaction::default(),
                    ))
                    .with_children(|b| {
                        if let Some(icon) = icon {
                            let s = &icon.sprite;
                            b.spawn((
                                ImageNode {
                                    image: icon.image.clone(),
                                    rect: Some(Rect::new(
                                        s.x as f32,
                                        s.y as f32,
                                        (s.x + s.width) as f32,
                                        (s.y + s.height) as f32,
                                    )),
                                    ..default()
                                },
                                Node { width: Val::Px(32.0), height: Val::Px(32.0), ..default() },
                            ));
                        }
                    });
                } else {
                    ghost_slot(row, &mut ctx, "empty_armor_slot");
                }
                for _ in 0..3 {
                    ghost_slot(row, &mut ctx, rest);
                }
            });
        }
    });
}

/// The shortcut bar: the game's `shortcut` prototypes in order, two rows filled column
/// by column; locked ones are greyed until their technology is researched.
pub(super) fn shortcut_bar(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    mut sprites: ResMut<Sprites>,
    root: Single<Entity, With<ShortcutBar>>,
    mut last: Local<Option<usize>>,
) {
    let r = sim.0.research();
    let researched = r.researched.iter().filter(|x| **x).count();
    if *last == Some(researched) {
        return;
    }
    *last = Some(researched);
    let db = sim.0.prototypes();
    let mut shortcuts: Vec<(String, String, factorio_data::RawValue)> = data
        .0
        .prototypes_in_category("shortcut")
        .map(|(_, name, p)| (p.get("order").as_str().unwrap_or("").to_owned(), name.to_owned(), p.clone()))
        .filter(|(_, _, p)| {
            // Hidden until unlocked, or not usable in this game (space age remotes).
            let tech = p.get("technology_to_unlock").as_str();
            let unlocked = tech.is_none_or(|t| db.technology_id(t).is_some_and(|id| r.is_researched(id)));
            unlocked || !p.get("unavailable_until_unlocked").as_bool().unwrap_or(false)
        })
        .collect();
    shortcuts.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    shortcuts.truncate(16);
    commands.entity(*root).despawn_related::<Children>();
    commands.entity(*root).with_children(|bar| {
        for column in shortcuts.chunks(2) {
            bar.spawn(Node { flex_direction: FlexDirection::Column, ..default() }).with_children(|col| {
                for (_, name, p) in column {
                    let tech = p.get("technology_to_unlock").as_str();
                    let unlocked = tech.is_none_or(|t| db.technology_id(t).is_some_and(|id| r.is_researched(id)));
                    let l = looks();
                    // ALT mode is shown toggled on, as the game starts with it.
                    let look = if name == "toggle-alt-mode" {
                        &l.yellow_slot
                    } else {
                        match p.get("style").as_str() {
                            Some("blue") => &l.shortcut_buttons[1],
                            Some("red") => &l.shortcut_buttons[2],
                            Some("green") => &l.shortcut_buttons[3],
                            _ => &l.shortcut_buttons[0],
                        }
                    };
                    let d = data.0.clone();
                    let icon = sprites.get(&assets, &data, &format!("shortcut:{name}"), || {
                        factorio_data::sprite::icon_of(&d, d.prototype("shortcut", name))
                    });
                    let mut node = crate::gui_skin::node_image(&look.default);
                    if !unlocked {
                        node.color = Color::srgba(1.0, 1.0, 1.0, 0.5);
                    }
                    col.spawn((
                        Node {
                            width: Val::Px(SLOT_PX),
                            height: Val::Px(SLOT_PX),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        node,
                        Interaction::default(),
                        Tip::Text(name.replace('-', " ")),
                    ))
                    .with_children(|b| {
                        if let Some(icon) = icon {
                            let s = &icon.sprite;
                            b.spawn((
                                ImageNode {
                                    image: icon.image.clone(),
                                    rect: Some(Rect::new(
                                        s.x as f32,
                                        s.y as f32,
                                        (s.x + s.width) as f32,
                                        (s.y + s.height) as f32,
                                    )),
                                    color: if unlocked { Color::WHITE } else { Color::srgba(1.0, 1.0, 1.0, 0.35) },
                                    ..default()
                                },
                                Node { width: Val::Px(24.0), height: Val::Px(24.0), ..default() },
                                Pickable::IGNORE,
                            ));
                        }
                    });
                }
            });
        }
    });
}

/// Redraws the minimap a few times a second: tiles in their map colours, resources,
/// trees and buildings in the game's chart colours, centred on the character.
pub(super) fn minimap(
    sim: Res<Sim>,
    data: Res<Data>,
    mut map: ResMut<Minimap>,
    mut images: ResMut<Assets<Image>>,
    mut frame: Local<u32>,
) {
    *frame += 1;
    if *frame % 20 != 1 {
        return;
    }
    let Some(c) = character(&sim) else { return };
    let centre = TilePosition::new(c.x.floor_int() as i32, c.y.floor_int() as i32);
    let db = sim.0.prototypes();
    let surface = &sim.0.surface;
    let n = MINIMAP_PX as i32;
    let mut px = vec![0u8; (MINIMAP_PX * MINIMAP_PX * 4) as usize];
    let chart = data.0.prototype("utility-constants", "default").get("chart").clone();
    let color4 = |v: &factorio_data::RawValue| -> Option<[u8; 4]> {
        let ch = |i: usize, k: &str| v.at(i).as_f64().or(v.get(k).as_f64());
        let (r, g, b) = (ch(0, "r")?, ch(1, "g")?, ch(2, "b")?);
        let a = ch(3, "a").unwrap_or(1.0);
        let k = if r > 1.0 || g > 1.0 || b > 1.0 { 1.0 } else { 255.0 };
        let ka = if a > 1.0 { 1.0 } else { 255.0 };
        Some([(r * k) as u8, (g * k) as u8, (b * k) as u8, (a * ka) as u8])
    };
    for y in 0..n {
        for x in 0..n {
            let t = TilePosition::new(centre.x + x - n / 2, centre.y + y - n / 2);
            let mut col = match surface.tile(t) {
                Some(tile) => {
                    let c = db.tile(tile).map_color;
                    [c[0], c[1], c[2]]
                }
                None => [0, 0, 0],
            };
            if let Some(res) = surface.resource(t) {
                let proto = db.entity(res.proto);
                let entry = map
                    .colors
                    .entry(res.proto)
                    .or_insert_with(|| color4(data.0.prototype(&proto.kind, &proto.name).get("map_color")));
                if let Some(c) = entry {
                    col = [c[0], c[1], c[2]];
                }
            }
            let i = ((y * n + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&[col[0], col[1], col[2], 255]);
        }
    }
    // Entities on top.
    let area = factorio_sim::map::Area {
        left_top: MapPosition::tile_center(TilePosition::new(centre.x - n / 2, centre.y - n / 2)),
        right_bottom: MapPosition::tile_center(TilePosition::new(centre.x + n / 2, centre.y + n / 2)),
    };
    for id in sim.0.entities_in(area) {
        let Some(e) = sim.0.entity(id) else { continue };
        let proto = db.entity(e.proto);
        let c = *map.colors.entry(e.proto).or_insert_with(|| {
            let raw = data.0.prototype(&proto.kind, &proto.name);
            color4(raw.get("friendly_map_color"))
                .or_else(|| color4(raw.get("map_color")))
                .or_else(|| color4(chart.get("default_friendly_color_by_type").get(&proto.kind)))
                .or_else(|| color4(chart.get("default_color_by_type").get(&proto.kind)))
                .or_else(|| {
                    // Rocks and other scenery are not charted; buildings default to blue.
                    if matches!(e.state, factorio_sim::world::EntityState::Static { .. }) {
                        None
                    } else {
                        color4(chart.get("default_friendly_color"))
                    }
                })
        });
        let Some(c) = c else { continue };
        let t = e.position.tile();
        let (x, y) = (t.x - centre.x + n / 2, t.y - centre.y + n / 2);
        if x < 0 || y < 0 || x >= n || y >= n {
            continue;
        }
        let i = ((y * n + x) * 4) as usize;
        let a = c[3] as u32;
        for k in 0..3 {
            px[i + k] = ((px[i + k] as u32 * (255 - a) + c[k] as u32 * a) / 255) as u8;
        }
    }
    // The character: a white dot.
    for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
        let (x, y) = (n / 2 + dx, n / 2 + dy);
        let i = ((y * n + x) * 4) as usize;
        px[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
    }
    if let Some(image) = images.get_mut(&map.image) {
        image.data = Some(px);
    }
}
