//! The settings panel (Escape when no window is open). For now it holds the sound volumes,
//! which start from the player's Factorio settings and are saved as this game's own.
//! A full settings menu comes with the main menu (roadmap step 4).

use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use crate::controls::UiState;
use crate::sound::Settings;
use crate::ui::Fonts;

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, setup).add_systems(Update, update);
    }
}

const BAR_PX: f32 = 260.0;

#[derive(Component)]
struct SettingsRoot;
#[derive(Component)]
struct Slider(usize);
#[derive(Component)]
struct Fill(usize);
#[derive(Component)]
struct ValueText(usize);

fn setup(mut commands: Commands, fonts: Res<Fonts>, mut settings: ResMut<Settings>) {
    let font = |size: f32| TextFont { font: fonts.regular.clone(), font_size: size, ..default() };
    let heading = TextFont { font: fonts.bold.clone(), font_size: 20.0, ..default() };
    let labels: Vec<&'static str> = settings.0.sliders_mut().iter().map(|(l, _)| *l).collect();
    commands
        .spawn((
            SettingsRoot,
            Interaction::default(),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(20.0),
                left: Val::Percent(50.0),
                margin: UiRect::left(Val::Px(-230.0)),
                width: Val::Px(460.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                padding: UiRect::all(Val::Px(14.0)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgb(0.192, 0.188, 0.192)),
            GlobalZIndex(30),
        ))
        .with_children(|p| {
            p.spawn((Text::new("Settings"), heading.clone(), TextColor(Color::srgb(1.0, 0.902, 0.753))));
            p.spawn((Text::new("Sound"), font(16.0), TextColor(Color::srgb(1.0, 0.902, 0.753))));
            for (i, label) in labels.iter().enumerate() {
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(10.0),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new(*label),
                        font(15.0),
                        TextColor(Color::WHITE),
                        Node { width: Val::Px(120.0), ..default() },
                    ));
                    row.spawn((
                        Slider(i),
                        Interaction::default(),
                        RelativeCursorPosition::default(),
                        Node { width: Val::Px(BAR_PX), height: Val::Px(14.0), ..default() },
                        BackgroundColor(Color::srgb(0.08, 0.08, 0.08)),
                    ))
                    .with_children(|bar| {
                        bar.spawn((
                            Fill(i),
                            Node { width: Val::Percent(0.0), height: Val::Percent(100.0), ..default() },
                            BackgroundColor(Color::srgb(0.95, 0.62, 0.15)),
                        ));
                    });
                    row.spawn((ValueText(i), Text::new(""), font(15.0), TextColor(Color::WHITE)));
                });
            }
            p.spawn((
                Text::new("Starts from your Factorio settings; changes are saved for this game only.\nEsc closes."),
                font(13.0),
                TextColor(Color::srgb(0.6, 0.6, 0.6)),
            ));
        });
}

fn update(
    ui: Res<UiState>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut settings: ResMut<Settings>,
    mut root: Single<&mut Node, With<SettingsRoot>>,
    sliders: Query<(&Slider, &Interaction, &RelativeCursorPosition)>,
    mut fills: Query<(&Fill, &mut Node), Without<SettingsRoot>>,
    mut texts: Query<(&ValueText, &mut Text)>,
    mut dirty: Local<bool>,
) {
    root.display = if ui.settings_open { Display::Flex } else { Display::None };
    if !ui.settings_open {
        return;
    }
    for (slider, interaction, cursor) in &sliders {
        if *interaction == Interaction::Pressed
            && let Some(p) = cursor.normalized
        {
            let v = ((p.x + 0.5).clamp(0.0, 1.0) * 100.0).round() / 100.0;
            let s = settings.0.sliders_mut();
            if *s[slider.0].1 != v {
                *s[slider.0].1 = v;
                *dirty = true;
            }
        }
    }
    let values: Vec<f32> = settings.0.sliders_mut().iter().map(|(_, v)| **v).collect();
    for (fill, mut node) in &mut fills {
        node.width = Val::Percent(values[fill.0] * 100.0);
    }
    for (t, mut text) in &mut texts {
        text.0 = format!("{:.0}%", values[t.0] * 100.0);
    }
    if *dirty && mouse.just_released(MouseButton::Left) {
        *dirty = false;
        if let Err(e) = settings.0.save() {
            warn!("could not save sound settings: {e}");
        }
    }
}
