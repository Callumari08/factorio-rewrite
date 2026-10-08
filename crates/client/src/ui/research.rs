//! Research GUI: the technology window (T), the current research in the top left, and the
//! research part of lab windows.

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use factorio_sim::proto::{ResearchTrigger, TechEffect, TechId};
use factorio_sim::research::Research;

use super::*;

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
        // The technology screen covers the whole window, as in the game.
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            flex_direction: FlexDirection::Row,
            display: Display::None,
            ..default()
        },
        GlobalZIndex(5),
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

/// Technology slot size (the game's `technology_slot`) and its pack band.
const SLOT_W: f32 = 72.0;
const SLOT_H: f32 = 100.0;
const BAND_H: f32 = 28.0;
/// The selected technology's picture in the card and the tree.
const BIG_W: f32 = 136.0;
const BIG_H: f32 = 200.0;
/// Width of the left column of the technology screen.
const LEFT_W: f32 = 536.0;

/// A technology's state, as the slot colours show it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TechState {
    Available,
    /// Not yet available, but every prerequisite is researched or queued.
    Conditional,
    Unavailable,
    Researched,
}

fn tech_state(db: &PrototypeDb, r: &Research, t: TechId) -> TechState {
    if r.is_researched(t) {
        TechState::Researched
    } else if r.is_available(db, t) {
        TechState::Available
    } else if db.technology(t).prerequisites.iter().all(|p| r.is_researched(*p) || r.queue.contains(p)) {
        TechState::Conditional
    } else {
        TechState::Unavailable
    }
}

fn state_name(s: TechState) -> &'static str {
    match s {
        TechState::Available => "Available",
        TechState::Conditional => "Conditionally available",
        TechState::Unavailable => "Unavailable",
        TechState::Researched => "Researched",
    }
}

impl Ctx<'_> {
    /// A technology slot: the picture on the state's colour, the packs it costs in the
    /// darker band below, its level for levelled technologies.
    fn tech_slot(&mut self, p: &mut ChildSpawnerCommands, t: TechId, w: f32, h: f32, button: Option<UiButton>) {
        let r = self.research;
        let db = self.db;
        let state = tech_state(db, r, t);
        let look = &looks().tech_slots[state as usize];
        let mut e = p.spawn((
            Node {
                width: Val::Px(w),
                height: Val::Px(h),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                flex_shrink: 0.0,
                ..default()
            },
            crate::gui_skin::node_image(&look.default),
            look.clone(),
            Tip::Tech(t),
        ));
        match button {
            Some(b) => e.insert((b, Button)),
            None => e.insert(Interaction::default()),
        };
        let band = h * BAND_H / SLOT_H;
        e.with_children(|c| {
            c.spawn(Node {
                flex_grow: 1.0,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|pic| self.tech_icon(pic, t, (h - band) * 0.92));
            // The band: darker, with the science packs.
            c.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(band),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::FlexEnd,
                    padding: UiRect { left: Val::Px(4.0), bottom: Val::Px(2.0), ..default() },
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
                Pickable::IGNORE,
            ))
            .with_children(|b| {
                if let Some(unit) = &db.technology(t).unit {
                    for (pack, _) in &unit.ingredients {
                        self.icon(b, *pack, 16.0);
                    }
                }
            });
            if names_level(db, t) {
                c.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(4.0),
                        bottom: Val::Px(band),
                        padding: UiRect::axes(Val::Px(6.0), Val::Px(0.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.95, 0.88, 0.88)),
                    Pickable::IGNORE,
                ))
                .with_children(|l| {
                    l.spawn((
                        Text::new(r.level[t.index()].to_string()),
                        TextFont { font: self.fonts.bold.clone(), font_size: 11.0, ..default() },
                        TextColor(Color::BLACK),
                    ));
                });
            }
        });
    }

    /// A section heading of the technology screen (the game's `heading-1`).
    fn screen_heading(&self, p: &mut ChildSpawnerCommands, s: impl Into<String>) {
        p.spawn((
            Text::new(s.into()),
            TextFont { font: self.fonts.bold.clone(), font_size: 18.0, ..default() },
            TextColor(looks().title_color),
        ));
    }
}

/// Whether a technology shows a level (levelled technologies).
fn names_level(db: &PrototypeDb, t: TechId) -> bool {
    let tech = db.technology(t);
    tech.max_level.is_some_and(|m| m > 1) || tech.name.ends_with(char::is_numeric)
}

/// The technology screen.
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
    // Selected: the chosen one, else the current research, else the first available.
    let mut memo = HashMap::new();
    let mut order: Vec<TechId> = db.technology_ids().filter(|t| visible(db, *t)).collect();
    order.sort_by_cached_key(|t| {
        let state = match tech_state(db, r, *t) {
            TechState::Researched => 3,
            TechState::Available => 0,
            TechState::Conditional => 1,
            TechState::Unavailable => 2,
        };
        (state, depth(db, *t, &mut memo), db.technology(*t).order.clone(), db.technology(*t).name.clone())
    });
    let selected = tech.selected.filter(|t| visible(db, *t)).or(r.current()).or_else(|| order.first().copied());

    let root_id = root.0;
    commands.entity(root_id).despawn_related::<Children>();
    commands.entity(root_id).with_children(|w| {
        // Left: queue, the selected technology, and the list of technologies.
        w.spawn((
            Node {
                width: Val::Px(LEFT_W + 20.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                padding: UiRect { left: Val::Px(8.0), right: Val::Px(12.0), top: Val::Px(8.0), bottom: Val::Px(8.0) },
                ..default()
            },
            BackgroundColor(FRAME),
            Interaction::default(),
        ))
        .with_children(|left| {
            ctx.screen_heading(left, "Research queue");
            left.spawn(Node {
                height: Val::Px(SLOT_H + 16.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceEvenly,
                ..default()
            })
            .with_children(|q| {
                crate::gui_skin::backdrop(q, &looks().deep_in_shallow);
                for i in 0..7 {
                    match r.queue.get(i) {
                        Some(t) => ctx.tech_slot(q, *t, SLOT_W * 0.8, SLOT_H * 0.86, Some(UiButton::SelectTech(*t))),
                        None => {
                            q.spawn((
                                Node { width: Val::Px(58.0), height: Val::Px(86.0), ..default() },
                                BackgroundColor(Color::srgb(0.13, 0.13, 0.13)),
                                Outline::new(Val::Px(1.0), Val::ZERO, Color::srgb(0.2, 0.2, 0.2)),
                            ));
                        }
                    }
                }
            });
            if let Some(t) = selected {
                card(left, &mut ctx, &names, t);
            }
            left.spawn(Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, ..default() })
                .with_children(|h| {
                    h.spawn(Node { flex_grow: 1.0, ..default() })
                        .with_children(|t| ctx.screen_heading(t, "List of technologies"));
                    let l = looks();
                    h.spawn((
                        Node {
                            width: Val::Px(28.0),
                            height: Val::Px(28.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        crate::gui_skin::node_image(&l.frame_button.default),
                        l.frame_button.clone(),
                        Button,
                        Tip::Text("Search".into()),
                    ))
                    .with_children(|b| ctx.utility(b, "search", 16.0));
                });
            left.spawn((
                TechList,
                ScrollPosition(Vec2::new(0.0, tech.scroll)),
                Node { flex_grow: 1.0, min_height: Val::Px(0.0), overflow: Overflow::scroll_y(), ..default() },
            ))
            .with_children(|list| {
                crate::gui_skin::backdrop(list, &looks().deep_in_shallow);
                list.spawn(Node {
                    display: Display::Grid,
                    grid_template_columns: RepeatedGridTrack::px(7, SLOT_W),
                    align_content: AlignContent::Start,
                    ..default()
                })
                .with_children(|g| {
                    for t in &order {
                        ctx.tech_slot(g, *t, SLOT_W, SLOT_H, Some(UiButton::SelectTech(*t)));
                    }
                });
            });
        });
        // Right: the technology tree around the selected technology, over the dimmed world.
        w.spawn((Node { flex_grow: 1.0, flex_direction: FlexDirection::Column, ..default() }, Interaction::default()))
            .with_children(|right| {
                right
                    .spawn((
                        Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)), ..default() },
                        BackgroundColor(FRAME),
                    ))
                    .with_children(|h| ctx.screen_heading(h, "Technology tree"));
                right.spawn((Node { height: Val::Px(36.0), ..default() }, BackgroundColor(INNER)));
                right
                    .spawn((
                        Node { flex_grow: 1.0, overflow: Overflow::clip(), ..default() },
                        // Blending is linear: 0.85 darkens the world to about 40 % brightness.
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.85)),
                    ))
                    .with_children(|canvas| {
                        if let Some(t) = selected {
                            tree(canvas, &mut ctx, t);
                        }
                    });
            });
    });
}

/// The selected technology's card: its picture, cost, effects and description, and the
/// start/remove button.
fn card(p: &mut ChildSpawnerCommands, ctx: &mut Ctx, names: &Names, t: TechId) {
    let db = ctx.db;
    let r = ctx.research;
    let tech = db.technology(t);
    ctx.screen_heading(p, format!("{} ({})", names.tech(r, t), state_name(tech_state(db, r, t))));
    p.spawn(Node { flex_direction: FlexDirection::Column, ..default() }).with_children(|card| {
        crate::gui_skin::backdrop(card, &looks().tech_card);
        card.spawn(Node { flex_direction: FlexDirection::Row, ..default() }).with_children(|row| {
            row.spawn(Node { padding: UiRect::all(Val::Px(4.0)), ..default() })
                .with_children(|pic| ctx.tech_slot(pic, t, BIG_W, BIG_H, None));
            row.spawn(Node {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            })
            .with_children(|col| {
                let sub = |col: &mut ChildSpawnerCommands, ctx: &Ctx, s: &str| {
                    col.spawn((
                        Text::new(s),
                        TextFont { font: ctx.fonts.bold.clone(), font_size: 15.0, ..default() },
                        TextColor(looks().title_color),
                    ));
                };
                if let Some(unit) = &tech.unit {
                    sub(col, ctx, "Cost");
                    let count = unit.count_for(r.level[t.index()]);
                    col.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(12.0),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn((
                            Node {
                                flex_direction: FlexDirection::Row,
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(6.0),
                                padding: UiRect::all(Val::Px(4.0)),
                                ..default()
                            },
                            crate::gui_skin::node_image(&looks().deep_in_shallow),
                        ))
                        .with_children(|box_| {
                            for (pack, amount) in &unit.ingredients {
                                ctx.slot(box_, Some(*pack), Some(*amount), SLOT, None, None);
                            }
                            ctx.utility(box_, "clock", 12.0);
                            ctx.text(box_, format!("{}", unit.time_ticks as f64 / 60.0), 14.0, Color::WHITE);
                        });
                        ctx.text(row, format!("× {count}"), 14.0, Color::WHITE);
                    });
                    if !r.is_researched(t) && r.progress[t.index()] > 0 {
                        progress_bar_live(
                            col,
                            r.progress_fraction(db, t).to_f64_lossy(),
                            PROGRESS,
                            Some(Live::Research),
                        );
                    }
                }
                if let Some(trigger) = &tech.trigger {
                    sub(col, ctx, "Researched by");
                    let mut text = trigger_text(names, db, trigger);
                    if let ResearchTrigger::CraftItem { count, .. } = trigger
                        && *count > 1
                        && !r.is_researched(t)
                    {
                        text += &format!("   ({}/{count})", r.trigger_counts[t.index()]);
                    }
                    ctx.text(col, text, 14.0, Color::WHITE);
                }
                separator(col);
                if !tech.effects.is_empty() {
                    sub(col, ctx, "Effects");
                    col.spawn(Node { flex_direction: FlexDirection::Row, flex_wrap: FlexWrap::Wrap, ..default() })
                        .with_children(|row| {
                            for e in &tech.effects {
                                if let TechEffect::UnlockRecipe(rec) = e {
                                    let main = db.recipe(*rec).results.first().and_then(|x| match x.what {
                                        ItemOrFluid::Item(i) => Some(i),
                                        _ => None,
                                    });
                                    if let Some(i) = main {
                                        row.spawn((
                                            Node { width: Val::Px(36.0), height: Val::Px(36.0), ..default() },
                                            Interaction::default(),
                                            Tip::Recipe(*rec),
                                        ))
                                        .with_children(|c| ctx.icon(c, i, 32.0));
                                    }
                                }
                            }
                        });
                    for e in &tech.effects {
                        match e {
                            TechEffect::Modifier { kind, qualifier, modifier } => {
                                let text = match names.modifiers.get(kind) {
                                    Some(s) if !s.contains("__1__") => s.clone(),
                                    _ => modifier_text(names, kind, qualifier, *modifier),
                                };
                                ctx.text(col, text, 14.0, Color::WHITE);
                            }
                            TechEffect::GiveItem { item, count } => {
                                ctx.text(col, format!("Gives {count} × {}", names.item(*item)), 14.0, Color::WHITE)
                            }
                            TechEffect::UnlockRecipe(_) => {}
                        }
                    }
                    separator(col);
                }
                if let Some(d) = names.tech_descriptions.get(&t) {
                    ctx.text(col, d.clone(), 14.0, Color::WHITE);
                }
            });
        });
        // Bottom bar with the button.
        card.spawn((
            Node {
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::FlexEnd,
                padding: UiRect::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.25)),
        ))
        .with_children(|bar| {
            if r.is_researched(t) || tech.unit.is_none() {
                return;
            }
            if let Some(block) = r.blocking_trigger(db, t) {
                let how =
                    db.technology(block).trigger.as_ref().map(|tr| trigger_text(names, db, tr)).unwrap_or_default();
                ctx.text(
                    bar,
                    format!("Needs {} first: {how}", names.tech(r, block)),
                    14.0,
                    Color::srgb(0.95, 0.5, 0.4),
                );
                return;
            }
            let (label, button) = if r.queue.contains(&t) {
                ("Remove from queue", UiButton::DequeueTech(t))
            } else {
                ("Start research", UiButton::QueueTech(t))
            };
            let l = looks();
            bar.spawn((
                Node { padding: UiRect::axes(Val::Px(12.0), Val::Px(4.0)), ..default() },
                crate::gui_skin::node_image(&l.button.default),
                l.button.clone(),
                button,
                Button,
                Tip::Text("Shift+click: put it at the front of the queue".into()),
            ))
            .with_children(|b| ctx.text(b, label, 14.0, Color::BLACK));
        });
    });
}

/// The tree around `t`: its prerequisites above (every level), `t` large, and the
/// technologies that need it below (three levels), joined by lines.
fn tree(canvas: &mut ChildSpawnerCommands, ctx: &mut Ctx, t: TechId) {
    let db = ctx.db;
    // Rows: 0 is `t`; negative rows prerequisites, positive rows dependents.
    let mut row_of: HashMap<TechId, i32> = HashMap::new();
    row_of.insert(t, 0);
    let mut frontier = vec![t];
    while let Some(x) = frontier.pop() {
        let row = row_of[&x];
        for p in &db.technology(x).prerequisites {
            if !visible(db, *p) {
                continue;
            }
            let e = row_of.entry(*p).or_insert(row - 1);
            if *e > row - 1 {
                *e = row - 1;
            }
            frontier.push(*p);
        }
    }
    let children = |x: TechId| -> Vec<TechId> {
        db.technology_ids().filter(|c| visible(db, *c) && db.technology(*c).prerequisites.contains(&x)).collect()
    };
    let mut level = vec![t];
    for depth in 1..=3 {
        let mut next = Vec::new();
        for x in &level {
            for c in children(*x) {
                if let std::collections::hash_map::Entry::Vacant(e) = row_of.entry(c) {
                    e.insert(depth);
                    next.push(c);
                }
            }
        }
        level = next;
    }
    // Columns: rows laid out left to right in a stable order, centred, each technology
    // placed near its relatives in the row before.
    let min_row = row_of.values().copied().min().unwrap_or(0);
    let max_row = row_of.values().copied().max().unwrap_or(0);
    let mut x_of: HashMap<TechId, f32> = HashMap::new();
    x_of.insert(t, 0.0);
    let pitch = SLOT_W + 8.0;
    let place_row =
        |row: i32, x_of: &mut HashMap<TechId, f32>, anchor: &dyn Fn(TechId, &HashMap<TechId, f32>) -> f32| {
            let mut techs: Vec<TechId> = row_of.iter().filter(|(_, r)| **r == row).map(|(k, _)| *k).collect();
            techs.sort_by(|a, b| {
                anchor(*a, x_of)
                    .partial_cmp(&anchor(*b, x_of))
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(db.technology(*a).order.cmp(&db.technology(*b).order))
                    .then(a.cmp(b))
            });
            let n = techs.len() as f32;
            let centre = if techs.is_empty() { 0.0 } else { techs.iter().map(|c| anchor(*c, x_of)).sum::<f32>() / n };
            for (i, c) in techs.iter().enumerate() {
                x_of.insert(*c, centre + (i as f32 - (n - 1.0) / 2.0) * pitch);
            }
        };
    for row in (min_row..0).rev() {
        // Above: under the technologies that need them.
        place_row(row, &mut x_of, &|c, xs| {
            let near: Vec<f32> = row_of
                .iter()
                .filter(|(k, r)| **r == row + 1 && db.technology(**k).prerequisites.contains(&c))
                .filter_map(|(k, _)| xs.get(k).copied())
                .collect();
            if near.is_empty() { 0.0 } else { near.iter().sum::<f32>() / near.len() as f32 }
        });
    }
    for row in 1..=max_row {
        place_row(row, &mut x_of, &|c, xs| {
            let near: Vec<f32> = db.technology(c).prerequisites.iter().filter_map(|p| xs.get(p).copied()).collect();
            if near.is_empty() { 0.0 } else { near.iter().sum::<f32>() / near.len() as f32 }
        });
    }
    // Vertical positions: 20 px between rows, the selected row taller.
    let row_y = |row: i32| -> f32 {
        if row < 0 {
            row as f32 * (SLOT_H + 20.0)
        } else if row == 0 {
            0.0
        } else {
            BIG_H + 20.0 + (row - 1) as f32 * (SLOT_H + 20.0)
        }
    };
    let size = |c: TechId| if c == t { (BIG_W, BIG_H) } else { (SLOT_W, SLOT_H) };
    // Centred horizontally; the selected technology just below its prerequisites.
    canvas
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            top: Val::Px(-row_y(min_row) + 24.0),
            ..default()
        })
        .with_children(|o| {
            let line = |o: &mut ChildSpawnerCommands, x: f32, y: f32, w: f32, h: f32| {
                o.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(x),
                        top: Val::Px(y),
                        width: Val::Px(w.max(1.0)),
                        height: Val::Px(h.max(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.62, 0.62, 0.62)),
                    Pickable::IGNORE,
                ));
            };
            // Lines: from a prerequisite's bottom to the technology's top, elbowed halfway.
            for (c, row) in &row_of {
                let (cw, _) = size(*c);
                let cx = x_of[c] + cw / 2.0 - pitch / 2.0 + (pitch - cw) / 2.0;
                let top = row_y(*row);
                for p in &db.technology(*c).prerequisites {
                    let Some(prow) = row_of.get(p) else { continue };
                    if *prow != row - 1 {
                        continue;
                    }
                    let (pw, ph) = size(*p);
                    let px = x_of[p] + pw / 2.0 - pitch / 2.0 + (pitch - pw) / 2.0;
                    let bottom = row_y(*prow) + ph;
                    let mid = (bottom + top) / 2.0;
                    line(o, px, bottom, 1.0, mid - bottom);
                    line(o, px.min(cx), mid, (px - cx).abs(), 1.0);
                    line(o, cx, mid, 1.0, top - mid);
                }
            }
            for (c, row) in &row_of {
                let (w, h) = size(*c);
                let x = x_of[c] - w / 2.0;
                o.spawn(Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(x),
                    top: Val::Px(row_y(*row)),
                    ..default()
                })
                .with_children(|n| ctx.tech_slot(n, *c, w, h, Some(UiButton::SelectTech(*c))));
            }
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
            Node { width: Val::Px(32.0), height: Val::Px(32.0), ..default() },
            UiButton::OpenTech,
            Button,
            Tip::Tech(t),
        ))
        .with_children(|c| ctx.tech_icon(c, t, 32.0));
        p.spawn(Node { flex_direction: FlexDirection::Column, flex_grow: 1.0, row_gap: Val::Px(2.0), ..default() })
            .with_children(|col| {
                col.spawn((
                    Text::new(if r.current() == Some(t) {
                        names.tech(r, t)
                    } else {
                        format!("Research completed: {}", names.tech(r, t))
                    }),
                    TextFont { font: ctx.fonts.bold.clone(), font_size: 14.0, ..default() },
                    TextColor(Color::WHITE),
                ));
                if r.current() == Some(t) {
                    // A thin bar with the percentage beside it.
                    col.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(8.0),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn(Node { flex_grow: 1.0, flex_direction: FlexDirection::Column, ..default() })
                            .with_children(|b| {
                                progress_bar_live(
                                    b,
                                    r.progress_fraction(db, t).to_f64_lossy(),
                                    PROGRESS,
                                    Some(Live::Research),
                                )
                            });
                        row.spawn((
                            Text::new(format!("{:.0}%", r.progress_fraction(db, t).to_f64_lossy() * 100.0)),
                            TextFont { font: ctx.fonts.regular.clone(), font_size: 12.0, ..default() },
                            TextColor(Color::WHITE),
                            LiveText(Live::Research),
                        ));
                    });
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
    // Science pack slots: an empty slot shows its pack pale, a filled one the opened
    // pack's durability as a green bar along its bottom.
    let n = lab.input.slots().len();
    slot_row(p, ctx, n, |row, ctx| {
        for (i, s) in lab.input.slots().iter().enumerate() {
            row.spawn(Node { width: Val::Px(SLOT_PX), height: Val::Px(SLOT_PX), ..default() }).with_children(|c| {
                ctx.slot(
                    c,
                    s.map(|s| s.item),
                    s.map(|s| s.count),
                    SLOT,
                    Some(UiButton::Slot(SlotRef::Opened(EntityInventory::Input, i as u16))),
                    inputs.get(i).map(|p| Tip::Item(*p)),
                );
                if s.is_none() {
                    if let Some(pack) = inputs.get(i) {
                        c.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(4.0),
                                top: Val::Px(4.0),
                                width: Val::Px(32.0),
                                height: Val::Px(32.0),
                                ..default()
                            },
                            Pickable::IGNORE,
                        ))
                        .with_children(|g| ctx.icon_tinted(g, *pack, 32.0, Color::srgba(1.0, 1.0, 1.0, 0.35)));
                    }
                } else {
                    c.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(4.0),
                            right: Val::Px(4.0),
                            bottom: Val::Px(3.0),
                            height: Val::Px(2.0),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Node {
                                width: Val::Percent((lab.opened_fraction(i).to_f64_lossy() * 100.0) as f32),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.2, 0.85, 0.2)),
                            LiveFill(Live::LabPack(i as u16)),
                        ));
                    });
                }
            });
        }
    });
    // The research being done: name and progress, and its picture on the right.
    p.spawn(Node { flex_direction: FlexDirection::Row, height: Val::Px(108.0), ..default() }).with_children(|area| {
        area.spawn(Node {
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            row_gap: Val::Px(8.0),
            padding: UiRect::all(Val::Px(12.0)),
            ..default()
        })
        .with_children(|left| {
            crate::gui_skin::backdrop(left, &looks().deep_in_shallow);
            if let Some(t) = r.current() {
                ctx.text(left, names.tech(r, t), 15.0, Color::WHITE);
                progress_bar_live(left, r.progress_fraction(db, t).to_f64_lossy(), PROGRESS, Some(Live::Research));
            }
        });
        area.spawn(Node {
            width: Val::Px(80.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|right| {
            crate::gui_skin::backdrop(right, &looks().deep_in_shallow);
            if let Some(t) = r.current() {
                ctx.tech_icon(right, t, 64.0);
            }
        });
    });
    module_row(p, ctx, proto);
}
