//! Research GUI: the technology window (T), the current research in the top left, and the
//! research part of lab windows.

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use factorio_sim::proto::{ResearchTrigger, TechEffect, TechId};
use factorio_sim::research::Research;

use super::*;

const RESEARCHED: Color = Color::srgb(0.24, 0.42, 0.20);
const AVAILABLE: Color = Color::srgb(0.55, 0.45, 0.16);
const LOCKED: Color = Color::srgb(0.36, 0.20, 0.18);
const QUEUED: Color = Color::srgb(0.20, 0.34, 0.52);
const CARD_PX: f32 = 76.0;

/// Which technology the window shows details for.
#[derive(Resource, Default)]
pub(super) struct TechUi {
    pub selected: Option<TechId>,
    scroll: f32,
}

#[derive(Component)]
pub(super) struct TechRoot;
#[derive(Component)]
pub(super) struct ResearchHudRoot;
#[derive(Component)]
pub(super) struct TechList;

pub(super) fn setup(mut commands: Commands, sim: Res<Sim>) {
    // `FACTORIO_REWRITE_TECH=<name>` selects a technology at start, for screenshots.
    let selected = std::env::var("FACTORIO_REWRITE_TECH").ok().and_then(|n| sim.0.prototypes().technology_id(&n));
    commands.insert_resource(TechUi { selected, scroll: 0.0 });
    commands.spawn((
        TechRoot,
        Interaction::default(),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(40.0),
            bottom: Val::Px(110.0),
            left: Val::Percent(50.0),
            width: Val::Px(1120.0),
            margin: UiRect::left(Val::Px(-560.0)),
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            padding: UiRect::all(Val::Px(8.0)),
            display: Display::None,
            ..default()
        },
        BackgroundColor(FRAME),
        GlobalZIndex(5),
    ));
    commands.spawn((
        ResearchHudRoot,
        Interaction::default(),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(84.0),
            left: Val::Px(8.0),
            width: Val::Px(172.0),
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(6.0),
            padding: UiRect::all(Val::Px(4.0)),
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(FRAME.with_alpha(0.85)),
    ));
}

impl Names {
    /// A technology's name with its level for levelled technologies ("Mining productivity 4").
    pub fn tech(&self, research: &Research, t: TechId) -> String {
        match self.techs.get(&t) {
            Some((name, true)) => format!("{name} {}", research.level[t.index()]),
            Some((name, false)) => name.clone(),
            None => "?".into(),
        }
    }
}

impl Ctx<'_> {
    pub(super) fn tech_icon(&mut self, p: &mut ChildSpawnerCommands, t: TechId, size: f32) {
        let name = self.db.technology(t).name.clone();
        let d = self.data.0.clone();
        let icon = self.sprites.get(self.assets, self.data, &format!("tech:{name}"), || {
            factorio_data::sprite::icon_of(&d, d.prototype("technology", &name))
        });
        if let Some(icon) = icon {
            let s = &icon.sprite;
            p.spawn((
                ImageNode {
                    image: icon.image.clone(),
                    rect: Some(Rect::new(s.x as f32, s.y as f32, (s.x + s.width) as f32, (s.y + s.height) as f32)),
                    ..default()
                },
                Node { width: Val::Px(size), height: Val::Px(size), ..default() },
            ));
        }
    }

    /// A clickable technology card coloured by its state.
    fn tech_card(&mut self, p: &mut ChildSpawnerCommands, t: TechId, selected: bool, size: f32) {
        let r = self.research;
        let bg = tech_color(self.db, r, t);
        let mut e = p.spawn((
            Node {
                width: Val::Px(size),
                height: Val::Px(size),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(if selected { 3.0 } else { 1.0 })),
                ..default()
            },
            BackgroundColor(bg),
            Base(bg),
            BorderColor::all(if selected { HEADING } else { Color::srgb(0.08, 0.08, 0.08) }),
            UiButton::SelectTech(t),
            Button,
            Tip::Tech(t),
        ));
        e.with_children(|c| self.tech_icon(c, t, size - 12.0));
    }
}

fn tech_color(db: &PrototypeDb, r: &Research, t: TechId) -> Color {
    if r.is_researched(t) {
        RESEARCHED
    } else if r.queue.contains(&t) {
        QUEUED
    } else if r.is_available(db, t) {
        AVAILABLE
    } else {
        LOCKED
    }
}

/// Technologies shown in the window: enabled and not hidden.
fn visible(db: &PrototypeDb, t: TechId) -> bool {
    let tech = db.technology(t);
    tech.enabled && !tech.hidden
}

/// Depth in the prerequisite graph, so the list reads roughly from early to late game.
fn depth(db: &PrototypeDb, t: TechId, memo: &mut HashMap<TechId, u32>) -> u32 {
    if let Some(d) = memo.get(&t) {
        return *d;
    }
    let d = db.technology(t).prerequisites.iter().map(|p| depth(db, *p, memo) + 1).max().unwrap_or(0);
    memo.insert(t, d);
    d
}

/// Text for a modifier effect, from the game's `modifier-description` strings.
fn modifier_text(names: &Names, kind: &str, qualifier: &str, modifier: factorio_sim::Fixed) -> String {
    let key = match kind {
        "ammo-damage" => format!("{qualifier}-damage-bonus"),
        "gun-speed" => format!("{qualifier}-shooting-speed-bonus"),
        "turret-attack" => format!("{qualifier}-attack-bonus"),
        _ => kind.to_owned(),
    };
    let integer = kind.ends_with("slots-bonus")
        || kind.ends_with("-count")
        || kind.ends_with("-storage")
        || kind.ends_with("capacity-bonus")
        || kind == "inserter-stack-size-bonus"
        || kind.ends_with("-distance");
    let v = modifier.to_f64_lossy();
    let value = if integer { format!("{v:.0}") } else { format!("{:.0}%", v * 100.0) };
    match names.modifiers.get(&key) {
        Some(text) => text.replace("__1__", &value),
        None => format!("{key}: +{value}"),
    }
}

fn trigger_text(names: &Names, db: &PrototypeDb, trigger: &ResearchTrigger) -> String {
    match trigger {
        ResearchTrigger::CraftItem { item, count } if *count > 1 => {
            format!("Craft {count} × {}", names.item(*item))
        }
        ResearchTrigger::CraftItem { item, .. } => format!("Craft {}", names.item(*item)),
        ResearchTrigger::MineEntity { entity } => format!("Mine {}", names.entity(*entity)),
        ResearchTrigger::BuildEntity { entity } => format!("Build {}", names.entity(*entity)),
        ResearchTrigger::CraftFluid { fluid, amount } => format!("Produce {amount} {}", db.fluid(*fluid).name),
        ResearchTrigger::Other(kind) => kind.clone(),
    }
}

/// Mouse wheel scrolls the technology list.
pub(super) fn scroll(
    scroll: Res<AccumulatedMouseScroll>,
    ui: Res<UiState>,
    mut tech: ResMut<TechUi>,
    mut list: Query<&mut ScrollPosition, With<TechList>>,
) {
    if !ui.tech_open || scroll.delta.y == 0.0 {
        return;
    }
    tech.scroll = (tech.scroll - scroll.delta.y * 40.0).max(0.0);
    for mut s in &mut list {
        s.0.y = tech.scroll;
        // The layout clamps the position; keep ours in step.
        tech.scroll = s.0.y.max(0.0).min(tech.scroll);
    }
}

/// The technology window.
pub(super) fn window(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    fonts: Res<Fonts>,
    names: Res<Names>,
    ui: Res<UiState>,
    tech: Res<TechUi>,
    mut sprites: ResMut<Sprites>,
    mut root: Single<(Entity, &mut Node), With<TechRoot>>,
    mut last: Local<String>,
) {
    root.1.display = if ui.tech_open { Display::Flex } else { Display::None };
    if !ui.tech_open {
        last.clear();
        return;
    }
    let r = sim.0.research();
    let db = sim.0.prototypes();
    let current_progress = r.current().map(|t| (r.progress_fraction(db, t).raw() >> 10, r.trigger_counts.clone()));
    let sig = format!(
        "{:?}|{:?}|{}|{:?}|{:?}",
        tech.selected,
        r.queue,
        r.researched.iter().filter(|x| **x).count(),
        current_progress,
        r.level,
    );
    if *last == sig {
        return;
    }
    *last = sig;
    let mut ctx =
        Ctx { sprites: &mut sprites, assets: &assets, data: &data, fonts: &fonts, db, research: r, hovered: None };
    let selected = tech.selected.filter(|t| visible(db, *t));

    // Available first, then locked by depth, then researched.
    let mut memo = HashMap::new();
    let mut order: Vec<TechId> = db.technology_ids().filter(|t| visible(db, *t)).collect();
    order.sort_by_cached_key(|t| {
        let state = if r.is_researched(*t) {
            2
        } else if r.is_available(db, *t) {
            0
        } else {
            1
        };
        (state, depth(db, *t, &mut memo), db.technology(*t).order.clone(), db.technology(*t).name.clone())
    });

    let root_id = root.0;
    commands.entity(root_id).despawn_related::<Children>();
    commands.entity(root_id).with_children(|w| {
        // Left: queue and the list of technologies.
        w.spawn((
            Node { width: Val::Px(640.0), flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), ..default() },
            BackgroundColor(INNER),
        ))
        .with_children(|left| {
            left.spawn(Node { padding: UiRect::all(Val::Px(8.0)), flex_direction: FlexDirection::Column, ..default() })
                .with_children(|q| {
                    ctx.heading(q, "Research queue");
                    if r.queue.is_empty() {
                        ctx.text(q, "Empty. Select a technology and press Start research.", 14.0, TEXT);
                    }
                    q.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(4.0), ..default() })
                        .with_children(|row| {
                            for (i, t) in r.queue.iter().enumerate() {
                                row.spawn(Node {
                                    flex_direction: FlexDirection::Column,
                                    width: Val::Px(60.0),
                                    ..default()
                                })
                                .with_children(|c| {
                                    ctx.tech_card(c, *t, selected == Some(*t), 60.0);
                                    if i == 0 {
                                        progress_bar(c, r.progress_fraction(db, *t).to_f64_lossy(), PROGRESS);
                                    }
                                });
                            }
                        });
                });
            left.spawn(Node { padding: UiRect::horizontal(Val::Px(8.0)), ..default() })
                .with_children(|h| ctx.heading(h, "Technologies"));
            left.spawn((
                TechList,
                ScrollPosition(Vec2::new(0.0, tech.scroll)),
                Node {
                    flex_grow: 1.0,
                    overflow: Overflow::scroll_y(),
                    padding: UiRect::all(Val::Px(8.0)),
                    ..default()
                },
            ))
            .with_children(|list| {
                list.spawn(Node {
                    display: Display::Grid,
                    grid_template_columns: RepeatedGridTrack::px(8, CARD_PX),
                    align_content: AlignContent::Start,
                    ..default()
                })
                .with_children(|g| {
                    for t in &order {
                        ctx.tech_card(g, *t, selected == Some(*t), CARD_PX);
                    }
                });
            });
        });

        // Right: details of the selected technology.
        w.spawn((
            Node {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(10.0)),
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(INNER),
        ))
        .with_children(|p| {
            let Some(t) = selected else {
                ctx.heading(p, "Select a technology");
                ctx.text(p, "Yellow: available   Blue: queued   Green: researched   Red: locked", 14.0, TEXT);
                return;
            };
            details(p, &mut ctx, &names, t);
        });
    });
}

fn details(p: &mut ChildSpawnerCommands, ctx: &mut Ctx, names: &Names, t: TechId) {
    let db = ctx.db;
    let r = ctx.research;
    let tech = db.technology(t);
    p.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(10.0), ..default() }).with_children(|row| {
        let bg = tech_color(db, r, t);
        row.spawn((
            Node { width: Val::Px(136.0), height: Val::Px(136.0), padding: UiRect::all(Val::Px(4.0)), ..default() },
            BackgroundColor(bg),
        ))
        .with_children(|c| ctx.tech_icon(c, t, 128.0));
        row.spawn(Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(4.0), flex_shrink: 1.0, ..default() })
            .with_children(|col| {
                ctx.heading(col, names.tech(r, t));
                if let Some(d) = names.tech_descriptions.get(&t) {
                    ctx.text(col, d.clone(), 14.0, TEXT);
                }
                let state = if r.is_researched(t) {
                    "Researched"
                } else if r.current() == Some(t) {
                    "Researching"
                } else if r.queue.contains(&t) {
                    "In the research queue"
                } else if r.is_available(db, t) {
                    "Available"
                } else {
                    "Requires other technologies"
                };
                ctx.text(col, state, 15.0, HEADING);
            });
    });

    // Cost.
    if let Some(unit) = &tech.unit {
        let count = unit.count_for(r.level[t.index()]);
        ctx.text(p, "Cost:", 15.0, HEADING);
        p.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(6.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            for (pack, amount) in &unit.ingredients {
                ctx.slot(row, Some(*pack), Some(*amount), SLOT, None, None);
            }
            ctx.text(row, format!("× {count}   {} s each", unit.time_ticks as f64 / 60.0), 15.0, TEXT);
        });
        if !r.is_researched(t) && r.progress[t.index()] > 0 {
            progress_bar_live(p, r.progress_fraction(db, t).to_f64_lossy(), PROGRESS, Some(Live::Research));
        }
    }
    if let Some(trigger) = &tech.trigger {
        ctx.text(p, "Researched by:", 15.0, HEADING);
        let mut text = trigger_text(names, db, trigger);
        if let ResearchTrigger::CraftItem { count, .. } = trigger
            && *count > 1
            && !r.is_researched(t)
        {
            text += &format!("   ({}/{count})", r.trigger_counts[t.index()]);
        }
        ctx.text(p, text, 15.0, TEXT);
    }

    // Effects.
    let recipes: Vec<RecipeId> = tech
        .effects
        .iter()
        .filter_map(|e| match e {
            TechEffect::UnlockRecipe(rec) => Some(*rec),
            _ => None,
        })
        .collect();
    if !tech.effects.is_empty() {
        ctx.text(p, "Effects:", 15.0, HEADING);
    }
    if !recipes.is_empty() {
        grid(p, 10, |g| {
            for rec in recipes {
                let main = db.recipe(rec).results.first().and_then(|x| match x.what {
                    ItemOrFluid::Item(i) => Some(i),
                    _ => None,
                });
                ctx.slot(g, main, None, SLOT, None, Some(Tip::Recipe(rec)));
            }
        });
    }
    for e in &tech.effects {
        match e {
            TechEffect::Modifier { kind, qualifier, modifier } => {
                let text = match names.modifiers.get(kind) {
                    Some(s) if !s.contains("__1__") => s.clone(),
                    _ => modifier_text(names, kind, qualifier, *modifier),
                };
                ctx.text(p, text, 14.0, TEXT);
            }
            TechEffect::GiveItem { item, count } => {
                ctx.text(p, format!("Gives {count} × {}", names.item(*item)), 14.0, TEXT)
            }
            TechEffect::UnlockRecipe(_) => {}
        }
    }

    // Prerequisites.
    if !tech.prerequisites.is_empty() {
        ctx.text(p, "Required technologies:", 15.0, HEADING);
        p.spawn(Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            column_gap: Val::Px(4.0),
            ..default()
        })
        .with_children(|row| {
            for pre in &tech.prerequisites {
                ctx.tech_card(row, *pre, false, 56.0);
            }
        });
    }

    // Queue buttons.
    if r.is_researched(t) || tech.unit.is_none() {
        return;
    }
    if let Some(block) = r.blocking_trigger(db, t) {
        let how = db.technology(block).trigger.as_ref().map(|tr| trigger_text(names, db, tr)).unwrap_or_default();
        ctx.text(p, format!("Needs {} first: {how}", names.tech(r, block)), 15.0, Color::srgb(0.95, 0.5, 0.4));
        return;
    }
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(8.0),
        margin: UiRect::top(Val::Px(8.0)),
        ..default()
    })
    .with_children(|row| {
        let (label, button) = if r.queue.contains(&t) {
            ("Remove from queue", UiButton::DequeueTech(t))
        } else {
            ("Start research", UiButton::QueueTech(t))
        };
        let bg = Color::srgb(0.30, 0.52, 0.22);
        row.spawn((
            Node { padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)), ..default() },
            BackgroundColor(bg),
            Base(bg),
            button,
            Button,
            Tip::Text("Shift+click: put it at the front of the queue".into()),
        ))
        .with_children(|b| ctx.text(b, label, 16.0, Color::WHITE));
    });
}

/// The current research, top left (Factorio shows it under the minimap).
pub(super) fn hud(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    fonts: Res<Fonts>,
    names: Res<Names>,
    mut sprites: ResMut<Sprites>,
    root: Single<Entity, With<ResearchHudRoot>>,
    mut last: Local<String>,
) {
    let r = sim.0.research();
    let db = sim.0.prototypes();
    let finished = r.last_finished.filter(|(_, tick)| sim.0.tick() < tick + 60 * 5);
    let sig = format!("{:?}|{:?}", r.current().map(|t| (t, r.progress_fraction(db, t).raw() >> 9)), finished);
    if *last == sig {
        return;
    }
    *last = sig;
    let mut ctx =
        Ctx { sprites: &mut sprites, assets: &assets, data: &data, fonts: &fonts, db, research: r, hovered: None };
    commands.entity(*root).despawn_related::<Children>();
    commands.entity(*root).with_children(|p| {
        let shown = r.current().or(finished.map(|(t, _)| t));
        let Some(t) = shown else {
            ctx.text(p, "No research in progress (T)", 14.0, TEXT);
            return;
        };
        p.spawn((
            Node { width: Val::Px(44.0), height: Val::Px(44.0), ..default() },
            UiButton::OpenTech,
            Button,
            Tip::Tech(t),
        ))
        .with_children(|c| ctx.tech_icon(c, t, 44.0));
        p.spawn(Node { flex_direction: FlexDirection::Column, flex_grow: 1.0, row_gap: Val::Px(2.0), ..default() })
            .with_children(|col| {
                if r.current() == Some(t) {
                    ctx.text(col, names.tech(r, t), 14.0, TEXT);
                    progress_bar(col, r.progress_fraction(db, t).to_f64_lossy(), PROGRESS);
                } else {
                    ctx.text(col, format!("Research completed: {}", names.tech(r, t)), 14.0, HEADING);
                }
            });
    });
}

/// The research section of a lab's window.
pub(super) fn lab_panel(
    p: &mut ChildSpawnerCommands,
    ctx: &mut Ctx,
    names: &Names,
    proto: &factorio_sim::proto::EntityProto,
    lab: &factorio_sim::research::LabState,
) {
    let inputs = match &proto.data {
        EntityData::Lab { inputs, .. } => inputs.clone(),
        _ => Vec::new(),
    };
    let r = ctx.research;
    let db = ctx.db;
    match r.current() {
        Some(t) => {
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|row| {
                ctx.tech_icon(row, t, 40.0);
                ctx.text(row, format!("Researching {}", names.tech(r, t)), 15.0, TEXT);
            });
            progress_bar_live(p, r.progress_fraction(db, t).to_f64_lossy(), PROGRESS, Some(Live::Research));
        }
        None => ctx.text(p, "No research in progress. Press T to choose one.", 14.0, TEXT),
    }
    ctx.text(p, "Science packs", 14.0, HEADING);
    grid(p, 10, |g| {
        for (i, s) in lab.input.slots().iter().enumerate() {
            g.spawn(Node { flex_direction: FlexDirection::Column, ..default() }).with_children(|c| {
                ctx.slot(
                    c,
                    s.map(|s| s.item),
                    s.map(|s| s.count),
                    SLOT,
                    Some(UiButton::Slot(SlotRef::Opened(EntityInventory::Input, i as u16))),
                    // Empty slots name the pack they take.
                    inputs.get(i).map(|p| Tip::Item(*p)),
                );
                // Durability left in the opened pack.
                progress_bar_live(
                    c,
                    lab.opened_fraction(i).to_f64_lossy(),
                    Color::srgb(0.3, 0.55, 0.85),
                    Some(Live::LabPack(i as u16)),
                );
            });
        }
    });
}
