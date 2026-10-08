//! The sound settings page (Escape when no window is open), laid out as the game's:
//! output and driver rows, then a checkbox, slider and value per sound category, the
//! reset button, and Back (undo the changes) / Confirm (save them). Volumes start from
//! the player's Factorio settings and are saved as this game's own.

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use factorio_data::sound::{SLIDERS, SoundSettings};

use crate::controls::UiState;
use crate::gui_skin::{looks, node_image};
use crate::sound::Settings;
use crate::ui::Fonts;

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, setup).add_systems(Update, update);
    }
}

/// Frame width, row height and slider length, as measured from the game.
const WIDTH: f32 = 555.0;
const ROW_H: f32 = 40.0;
const SLIDER_W: f32 = 160.0;

#[derive(Component)]
struct SettingsRoot;
#[derive(Component)]
struct Slider(usize);
#[derive(Component)]
struct Fill(usize);
#[derive(Component)]
struct SliderHandle(usize);
#[derive(Component)]
struct ValueText(usize);
#[derive(Component)]
struct Check(usize);
#[derive(Component)]
struct CheckMark(usize);
#[derive(Component, Clone, Copy, PartialEq)]
enum Action {
    Reset,
    Back,
    Confirm,
}

fn setup(
    mut commands: Commands,
    fonts: Res<Fonts>,
    data: Res<crate::Data>,
    assets: Res<AssetServer>,
    mut sprites: ResMut<crate::sprites::Sprites>,
) {
    let l = looks();
    let d = data.0.clone();
    // The dropdown arrow (the `dropdown` style's icon).
    let arrow: Option<Handle<Image>> = data
        .0
        .resolve_path("__core__/graphics/icons/mip/dropdown.png")
        .map(|p| assets.load(crate::sprites::asset_path(&data, &p)));
    let reset = sprites.get(&assets, &data, "utility:reset", || factorio_data::sprite::utility_sprite(&d, "reset"));
    let font = |size: f32| TextFont { font: fonts.regular.clone(), font_size: size, ..default() };
    let bold = |size: f32| TextFont { font: fonts.bold.clone(), font_size: size, ..default() };
    commands
        .spawn((
            SettingsRoot,
            Interaction::default(),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(12.0),
                left: Val::Percent(50.0),
                margin: UiRect::left(Val::Px(-WIDTH / 2.0)),
                width: Val::Px(WIDTH),
                flex_direction: FlexDirection::Column,
                padding: UiRect { left: Val::Px(8.0), right: Val::Px(8.0), top: Val::Px(4.0), bottom: Val::Px(8.0) },
                row_gap: Val::Px(8.0),
                display: Display::None,
                ..default()
            },
            node_image(&l.frame),
            GlobalZIndex(30),
        ))
        .with_children(|p| {
            // Title bar.
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                height: Val::Px(32.0),
                ..default()
            })
            .with_children(|h| {
                h.spawn((Text::new("Sound settings"), bold(18.0), TextColor(l.title_color)));
                let mut filler = h.spawn(Node { flex_grow: 1.0, height: Val::Px(24.0), ..default() });
                if let Some((image, rect)) = &l.header_filler {
                    filler.insert(ImageNode {
                        image: image.clone(),
                        rect: Some(*rect),
                        image_mode: bevy::ui::widget::NodeImageMode::Tiled {
                            tile_x: true,
                            tile_y: true,
                            stretch_value: l.scale,
                        },
                        ..default()
                    });
                }
            });
            // Subheader with the reset button.
            p.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::FlexEnd,
                    align_items: AlignItems::Center,
                    height: Val::Px(36.0),
                    padding: UiRect::right(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.14, 0.14, 0.14)),
            ))
            .with_children(|h| {
                h.spawn((
                    Action::Reset,
                    Button,
                    Node {
                        width: Val::Px(28.0),
                        height: Val::Px(28.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    node_image(&l.red_button.default),
                    l.red_button.clone(),
                ))
                .with_children(|b| {
                    if let Some(r) = &reset {
                        let s = &r.sprite;
                        b.spawn((
                            ImageNode {
                                image: r.image.clone(),
                                rect: Some(Rect::new(
                                    s.x as f32,
                                    s.y as f32,
                                    (s.x + s.width) as f32,
                                    (s.y + s.height) as f32,
                                )),
                                ..default()
                            },
                            Node { width: Val::Px(24.0), height: Val::Px(24.0), ..default() },
                            Pickable::IGNORE,
                        ));
                    }
                });
            });
            // The rows.
            p.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(4.0)),
                    row_gap: Val::Px(2.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.17, 0.17, 0.17)),
            ))
            .with_children(|rows| {
                let row_node = || {
                    (
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            height: Val::Px(ROW_H),
                            padding: UiRect::axes(Val::Px(12.0), Val::Px(0.0)),
                            column_gap: Val::Px(8.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.24, 0.24, 0.24)),
                    )
                };
                // Output and driver, shown as the game shows them; we always use the
                // default device.
                for (label, value, width) in
                    [("Preferred output", "Default device", 390.0), ("Preferred audio driver", "Default", 116.0)]
                {
                    rows.spawn(row_node()).with_children(|r| {
                        r.spawn((
                            Text::new(label),
                            font(15.0),
                            TextColor(Color::WHITE),
                            Node { flex_grow: 1.0, ..default() },
                        ));
                        r.spawn((
                            Node {
                                width: Val::Px(width),
                                height: Val::Px(28.0),
                                align_items: AlignItems::Center,
                                padding: UiRect { left: Val::Px(12.0), right: Val::Px(4.0), ..default() },
                                ..default()
                            },
                            node_image(&l.button.default),
                        ))
                        .with_children(|d| {
                            d.spawn((
                                Text::new(value),
                                bold(15.0),
                                TextColor(Color::BLACK),
                                Node { flex_grow: 1.0, ..default() },
                            ));
                            if let Some(a) = &arrow {
                                d.spawn((
                                    ImageNode {
                                        image: a.clone(),
                                        rect: Some(Rect::new(0.0, 0.0, 32.0, 32.0)),
                                        ..default()
                                    },
                                    Node { width: Val::Px(16.0), height: Val::Px(16.0), ..default() },
                                ));
                            }
                        });
                    });
                }
                for (i, (label, ..)) in SLIDERS.iter().enumerate() {
                    rows.spawn(row_node()).with_children(|r| {
                        r.spawn((
                            Check(i),
                            Button,
                            Node { width: Val::Px(14.0), height: Val::Px(14.0), ..default() },
                            node_image(&l.checkbox_checked),
                        ))
                        .with_children(|c| {
                            c.spawn((
                                CheckMark(i),
                                Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
                                node_image(&l.checkmark),
                                Pickable::IGNORE,
                            ));
                        });
                        r.spawn((
                            Text::new(*label),
                            font(15.0),
                            TextColor(Color::WHITE),
                            Node { flex_grow: 1.0, ..default() },
                        ));
                        r.spawn((
                            Slider(i),
                            Interaction::default(),
                            RelativeCursorPosition::default(),
                            Node {
                                width: Val::Px(SLIDER_W),
                                height: Val::Px(12.0),
                                align_items: AlignItems::Center,
                                ..default()
                            },
                        ))
                        .with_children(|bar| {
                            bar.spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(0.0),
                                    right: Val::Px(0.0),
                                    height: Val::Px(4.0),
                                    ..default()
                                },
                                ImageNode {
                                    image: l.slider_empty.image.clone(),
                                    rect: Some(l.slider_empty.rect),
                                    ..default()
                                },
                                Pickable::IGNORE,
                            ));
                            bar.spawn((
                                Fill(i),
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(0.0),
                                    width: Val::Percent(0.0),
                                    height: Val::Px(12.0),
                                    ..default()
                                },
                                node_image(&l.slider_full),
                                Pickable::IGNORE,
                            ));
                            bar.spawn((
                                SliderHandle(i),
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(0.0),
                                    width: Val::Px(20.0),
                                    height: Val::Px(12.0),
                                    ..default()
                                },
                                ImageNode {
                                    image: l.slider_handle.image.clone(),
                                    rect: Some(l.slider_handle.rect),
                                    ..default()
                                },
                                Pickable::IGNORE,
                            ));
                        });
                        r.spawn((
                            Node {
                                width: Val::Px(80.0),
                                height: Val::Px(28.0),
                                margin: UiRect::left(Val::Px(8.0)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            node_image(&l.textbox),
                        ))
                        .with_children(|t| {
                            t.spawn((ValueText(i), Text::new(""), font(14.0), TextColor(Color::BLACK)));
                        });
                    });
                }
            });
            // Back and Confirm.
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                height: Val::Px(32.0),
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|b| {
                dialog_button(b, &fonts, "Back", Action::Back, false);
                b.spawn(Node { flex_grow: 1.0, ..default() });
                dialog_button(b, &fonts, "Confirm", Action::Confirm, true);
            });
        });
}

/// The game's back (grey, arrow on the left) and confirm (green, arrow on the right)
/// buttons: the arrow half of the diamond picture and its stretched middle.
fn dialog_button(p: &mut ChildSpawnerCommands, fonts: &Fonts, label: &str, action: Action, green: bool) {
    let l = looks();
    let src = if green { l.dialog_green } else { l.dialog_grey };
    let half = |left: bool| {
        let x = if left { src.min.x } else { src.min.x + 24.0 };
        ImageNode { image: l.tileset.clone(), rect: Some(Rect::new(x, src.min.y, x + 24.0, src.max.y)), ..default() }
    };
    let middle = ImageNode {
        image: l.tileset.clone(),
        rect: Some(Rect::new(src.min.x + 24.0, src.min.y, src.min.x + 25.0, src.max.y)),
        ..default()
    };
    p.spawn((action, Button, Node { flex_direction: FlexDirection::Row, height: Val::Px(32.0), ..default() }))
        .with_children(|b| {
            if !green {
                b.spawn((
                    half(true),
                    Node { width: Val::Px(12.0), height: Val::Px(32.0), ..default() },
                    Pickable::IGNORE,
                ));
            }
            b.spawn((
                middle,
                Node {
                    width: Val::Px(110.0),
                    height: Val::Px(32.0),
                    justify_content: if green { JustifyContent::FlexEnd } else { JustifyContent::FlexStart },
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(0.0)),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|t| {
                t.spawn((
                    Text::new(label),
                    TextFont { font: fonts.bold.clone(), font_size: 18.0, ..default() },
                    TextColor(Color::BLACK),
                ));
            });
            if green {
                b.spawn((
                    half(false),
                    Node { width: Val::Px(12.0), height: Val::Px(32.0), ..default() },
                    Pickable::IGNORE,
                ));
            }
        });
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update(
    mut ui: ResMut<UiState>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut settings: ResMut<Settings>,
    mut root: Single<&mut Node, With<SettingsRoot>>,
    sliders: Query<(&Slider, &Interaction, &RelativeCursorPosition)>,
    checks: Query<(&Check, &Interaction), Changed<Interaction>>,
    actions: Query<(&Action, &Interaction), Changed<Interaction>>,
    mut fills: Query<(&Fill, &mut Node), (Without<SettingsRoot>, Without<SliderHandle>)>,
    mut handles: Query<(&SliderHandle, &mut Node), (Without<SettingsRoot>, Without<Fill>)>,
    mut marks: Query<(&CheckMark, &mut Visibility)>,
    mut texts: Query<(&ValueText, &mut Text)>,
    mut opened_with: Local<Option<SoundSettings>>,
) {
    root.display = if ui.settings_open { Display::Flex } else { Display::None };
    if !ui.settings_open {
        *opened_with = None;
        return;
    }
    // The values when the page was opened, for Back.
    let before = opened_with.get_or_insert_with(|| settings.0.clone()).clone();
    if mouse.pressed(MouseButton::Left) {
        for (slider, interaction, cursor) in &sliders {
            if *interaction == Interaction::Pressed
                && let Some(p) = cursor.normalized
            {
                settings.0.volumes[slider.0] = ((p.x + 0.5).clamp(0.0, 1.0) * 100.0).round() / 100.0;
            }
        }
    }
    for (check, interaction) in &checks {
        if *interaction == Interaction::Pressed {
            settings.0.enabled[check.0] = !settings.0.enabled[check.0];
        }
    }
    for (action, interaction) in &actions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            Action::Reset => settings.0 = SoundSettings::default(),
            Action::Back => {
                settings.0 = before.clone();
                ui.settings_open = false;
            }
            Action::Confirm => {
                if let Err(e) = settings.0.save() {
                    warn!("could not save sound settings: {e}");
                }
                ui.settings_open = false;
            }
        }
    }
    let s = &settings.0;
    for (fill, mut node) in &mut fills {
        node.width = Val::Percent(s.volumes[fill.0] * 100.0);
    }
    for (h, mut node) in &mut handles {
        node.left = Val::Px(s.volumes[h.0] * (SLIDER_W - 20.0));
    }
    for (m, mut v) in &mut marks {
        *v = if s.enabled[m.0] { Visibility::Inherited } else { Visibility::Hidden };
    }
    for (t, mut text) in &mut texts {
        text.0 = format!("{:.0}%", s.volumes[t.0] * 100.0);
    }
}
