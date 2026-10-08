//! Tooltips and the selected-entity info panel, in the game's tooltip style: a light title
//! band, "Key: value" lines with the key in the heading colour, and categories ("Consumes
//! Burnable fuel", "Generates electricity") with their own heading and line.

use factorio_sim::world::EntityState;

use super::*;

/// The game's tooltip key colour (`default_font_color` of tooltip headings).
const KEY: Color = Color::srgb(1.0, 0.902, 0.753);
/// `tooltip_heading_label_category`.
const CATEGORY: Color = Color::srgb(1.0, 0.8275, 0.29);
/// `tooltip_label`'s maximal width.
pub(super) const TIP_W: f32 = 356.0;

impl Ctx<'_> {
    fn tip_bold(&self, s: impl Into<String>, size: f32, color: Color) -> (Text, TextFont, TextColor) {
        (
            Text::new(s.into()),
            TextFont { font: self.fonts.bold.clone(), font_size: size, ..default() },
            TextColor(color),
        )
    }

    /// The light title band.
    pub(super) fn tip_title(&self, p: &mut ChildSpawnerCommands, title: impl Into<String>) {
        p.spawn((
            Node {
                padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                margin: UiRect::horizontal(Val::Px(-4.0)),
                ..default()
            },
            crate::gui_skin::node_image(&looks().tooltip_title),
        ))
        .with_children(|t| {
            t.spawn(self.tip_bold(title, 15.0, Color::BLACK));
        });
    }

    /// A "Key: value" line.
    pub(super) fn tip_kv(&self, p: &mut ChildSpawnerCommands, key: &str, value: impl Into<String>) {
        p.spawn((
            Text::default(),
            TextFont { font: self.fonts.bold.clone(), font_size: 14.0, ..default() },
            TextColor(KEY),
        ))
        .with_children(|t| {
            t.spawn((
                TextSpan::new(format!("{key}: ")),
                TextFont { font: self.fonts.bold.clone(), font_size: 14.0, ..default() },
                TextColor(KEY),
            ));
            t.spawn((
                TextSpan::new(value.into()),
                TextFont { font: self.fonts.regular.clone(), font_size: 14.0, ..default() },
                TextColor(Color::WHITE),
            ));
        });
    }

    /// A plain line.
    pub(super) fn tip_text(&self, p: &mut ChildSpawnerCommands, s: impl Into<String>) {
        p.spawn((
            Text::new(s.into()),
            TextFont { font: self.fonts.regular.clone(), font_size: 14.0, ..default() },
            TextColor(Color::WHITE),
            Node { max_width: Val::Px(TIP_W), ..default() },
        ));
    }

    /// A category heading: a separating line, then the heading in its colour.
    pub(super) fn tip_category(&self, p: &mut ChildSpawnerCommands, s: &str) {
        tip_line(p);
        p.spawn(self.tip_bold(s, 14.0, CATEGORY));
    }

    /// A key heading on its own ("Ingredients:").
    pub(super) fn tip_heading(&self, p: &mut ChildSpawnerCommands, s: &str) {
        p.spawn(self.tip_bold(s, 14.0, KEY));
    }

    /// An item icon with "N × Name", the amount bold.
    pub(super) fn tip_amount(
        &mut self,
        p: &mut ChildSpawnerCommands,
        item: Option<ItemId>,
        amount: String,
        name: String,
    ) {
        p.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(6.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            match item {
                Some(i) => self.icon(row, i, 24.0),
                None => {
                    row.spawn(Node { width: Val::Px(24.0), height: Val::Px(24.0), ..default() });
                }
            }
            row.spawn((Text::default(), TextFont { font: self.fonts.bold.clone(), font_size: 14.0, ..default() }))
                .with_children(|t| {
                    t.spawn((
                        TextSpan::new(format!("{amount} × ")),
                        TextFont { font: self.fonts.bold.clone(), font_size: 14.0, ..default() },
                        TextColor(Color::WHITE),
                    ));
                    t.spawn((
                        TextSpan::new(name),
                        TextFont { font: self.fonts.regular.clone(), font_size: 14.0, ..default() },
                        TextColor(Color::WHITE),
                    ));
                });
        });
    }
}

/// The tooltip's separating line (`tooltip_horizontal_line`).
pub(super) fn tip_line(p: &mut ChildSpawnerCommands) {
    p.spawn((
        Node { height: Val::Px(2.0), margin: UiRect::vertical(Val::Px(2.0)), ..default() },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
    ));
}

/// A tooltip or info panel body: the frame with the game's padding.
pub(super) fn tip_frame() -> (Node, ImageNode) {
    let mut image = crate::gui_skin::node_image(&looks().tooltip);
    image.color = image.color.with_alpha(0.88);
    (
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            padding: UiRect { left: Val::Px(4.0), right: Val::Px(4.0), top: Val::Px(0.0), bottom: Val::Px(4.0) },
            max_width: Val::Px(TIP_W + 8.0),
            ..default()
        },
        image,
    )
}

/// The contents of a tooltip.
pub(super) fn tooltip_contents(p: &mut ChildSpawnerCommands, ctx: &mut Ctx, names: &Names, sim: &Sim, tip: &Tip) {
    let db = ctx.db;
    match tip {
        Tip::Text(t) => {
            p.spawn(Node { padding: UiRect::top(Val::Px(4.0)), ..default() })
                .with_children(|b| ctx.tip_text(b, t.clone()));
        }
        Tip::Tech(t) => {
            let r = sim.0.research();
            ctx.tip_title(p, names.tech(r, *t));
            if let Some(d) = names.tech_descriptions.get(t) {
                ctx.tip_text(p, d.clone());
            }
            if let Some(unit) = &db.technology(*t).unit {
                let count = unit.count_for(r.level[t.index()]);
                ctx.tip_heading(p, "Cost:");
                for (pack, amount) in &unit.ingredients {
                    ctx.tip_amount(p, Some(*pack), (amount * count as u32).to_string(), names.item(*pack).to_owned());
                }
                ctx.tip_kv(p, "Time", format!("{} s", unit.time_ticks as u64 * count / 60));
            }
        }
        Tip::Item(i) => {
            let item = db.item(*i);
            ctx.tip_title(p, names.item(*i).to_owned());
            ctx.tip_kv(p, "Stack size", item.stack_size.to_string());
            if let Some(f) = &item.fuel {
                ctx.tip_category(p, "Fuel");
                ctx.tip_kv(p, "Fuel value", format!("{:.1} MJ", f.value.to_f64_lossy() / 1e6));
            }
            if item.place_result.is_some() {
                ctx.tip_category(p, "Placeable");
            }
        }
        Tip::Recipe(r) => {
            let rec = db.recipe(*r);
            ctx.tip_title(p, format!("{} (Recipe)", names.recipe(*r)));
            ctx.tip_heading(p, "Ingredients:");
            for ing in &rec.ingredients {
                match ing.what {
                    ItemOrFluid::Item(i) => {
                        ctx.tip_amount(p, Some(i), ing.amount.to_string(), names.item(i).to_owned())
                    }
                    ItemOrFluid::Fluid(f) => ctx.tip_amount(p, None, ing.amount.to_string(), db.fluid(f).name.clone()),
                }
            }
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|row| {
                row.spawn(Node { width: Val::Px(24.0), ..default() });
                ctx.utility(row, "clock", 12.0);
                row.spawn((Text::default(), TextFont { font: ctx.fonts.bold.clone(), font_size: 14.0, ..default() }))
                    .with_children(|t| {
                        t.spawn((
                            TextSpan::new(format!("{} s ", rec.energy_required)),
                            TextFont { font: ctx.fonts.bold.clone(), font_size: 14.0, ..default() },
                            TextColor(Color::WHITE),
                        ));
                        t.spawn((
                            TextSpan::new("Crafting time"),
                            TextFont { font: ctx.fonts.regular.clone(), font_size: 14.0, ..default() },
                            TextColor(Color::WHITE),
                        ));
                    });
            });
            if rec.results.len() > 1 || rec.results.first().is_some_and(|x| x.amount_max != factorio_sim::Fixed::ONE) {
                ctx.tip_heading(p, "Products:");
                for x in &rec.results {
                    if let ItemOrFluid::Item(i) = x.what {
                        ctx.tip_amount(p, Some(i), x.amount_max.to_string(), names.item(i).to_owned());
                    }
                }
            }
        }
    }
}

/// The info panel for the entity under the mouse (under the minimap, as in the game):
/// its picture, name, status with its light, the figures that matter for its type, and
/// what it consumes or generates.
pub(super) fn entity_info(p: &mut ChildSpawnerCommands, ctx: &mut Ctx, names: &Names, sim: &Sim, id: EntityId) {
    let db = ctx.db;
    let Some(e) = sim.0.entity(id) else { return };
    let proto = db.entity(e.proto);
    let raw = ctx.data.0.prototype(&proto.kind, &proto.name).clone();
    entity_picture(p, ctx, sim, id, Vec2::new(hud::SIDE_MENU_W, 104.0));
    // Title band, regular weight.
    p.spawn((
        Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)), ..default() },
        crate::gui_skin::node_image(&looks().tooltip_title),
    ))
    .with_children(|t| {
        t.spawn((
            Text::new(names.entity(e.proto).to_owned()),
            TextFont { font: ctx.fonts.regular.clone(), font_size: 15.0, ..default() },
            TextColor(Color::BLACK),
        ));
    });
    p.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(2.0),
        padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
        ..default()
    })
    .with_children(|p| {
        let (st, color) = status(sim, id);
        if !st.is_empty() {
            let light = if color == STATUS_GREEN {
                "status_working"
            } else if color == STATUS_RED {
                "status_not_working"
            } else {
                "status_yellow"
            };
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|r| {
                ctx.utility(r, light, 14.0);
                r.spawn((
                    Text::new(st),
                    TextFont { font: ctx.fonts.bold.clone(), font_size: 14.0, ..default() },
                    TextColor(Color::WHITE),
                ));
            });
        }
        if let EntityData::CraftingMachine { crafting_speed, .. } = &proto.data {
            ctx.tip_kv(p, "Crafting speed", trim(crafting_speed.to_f64_lossy()));
        }
        if let EntityState::Crafter(c) = &e.state
            && let Some(r) = c.recipe
        {
            ctx.tip_kv(p, "Recipe", names.recipe(r).to_owned());
        }
        let pollution = raw.get("energy_source").get("emissions_per_minute").get("pollution").as_f64();
        if let Some(v) = pollution {
            ctx.tip_kv(p, "Pollution", format!("{}/m", trim(v)));
        }
        if let EntityState::Container(inv) = &e.state {
            let used = inv.slots().iter().filter(|s| s.is_some()).count();
            ctx.tip_kv(p, "Inventory", format!("{used}/{}", inv.len()));
        }
        if let EntityState::Fluid(f) = &e.state {
            for b in &f.boxes {
                if let Some(fluid) = b.fluid {
                    ctx.tip_kv(p, &capitalise(&db.fluid(fluid).name), format!("{:.1}", b.amount.to_f64_lossy()));
                    ctx.tip_kv(p, "Temperature", format!("{:.2} °C", b.temperature.to_f64_lossy()));
                }
            }
        }
        // Health is not simulated yet: entities are always at full health.
        if let Some(h) = raw.get("max_health").as_f64() {
            ctx.tip_kv(p, "Health", format!("{}/{}", trim(h), trim(h)));
        }
    });
    let power = |v: factorio_sim::Fixed| crate::chart::power_text(v.to_f64_lossy());
    let section = |p: &mut ChildSpawnerCommands, f: &mut dyn FnMut(&mut ChildSpawnerCommands)| {
        p.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
            ..default()
        })
        .with_children(|c| f(c));
    };
    let energy = match &e.state {
        EntityState::Crafter(c) => Some(&c.energy),
        EntityState::Drill(d) => Some(&d.energy),
        EntityState::Inserter(i) => Some(&i.energy),
        EntityState::Lab(l) => Some(&l.energy),
        EntityState::Fluid(f) => Some(&f.energy),
        _ => None,
    };
    if let Some(b) = energy.and_then(|en| en.burner()) {
        section(p, &mut |c| {
            ctx.tip_category(c, "Consumes Burnable fuel");
            let usage = match &proto.data {
                EntityData::MiningDrill { energy_usage, .. } | EntityData::CraftingMachine { energy_usage, .. } => {
                    Some(*energy_usage)
                }
                EntityData::Boiler { energy_consumption, .. } => Some(*energy_consumption),
                _ => None,
            };
            if let Some(u) = usage {
                ctx.tip_kv(c, "Max. consumption", power(u));
            }
            for s in b.fuel.slots().iter().flatten() {
                ctx.tip_amount(c, Some(s.item), s.count.to_string(), names.item(s.item).to_owned());
            }
        });
    }
    if let Some(EnergySource::Electric { .. }) = proto.energy_source() {
        section(p, &mut |c| {
            ctx.tip_category(c, "Consumes electricity");
            let max = factorio_sim::power::electric_buffer_capacity(proto);
            ctx.tip_kv(c, "Max. consumption", power(max));
            // The game's drain: a thirtieth of the usage unless the prototype says.
            let drain = raw
                .get("energy_source")
                .get("drain")
                .as_str()
                .map(|s| s.to_owned())
                .unwrap_or_else(|| power(max / factorio_sim::Fixed::from_int(31)));
            ctx.tip_kv(c, "Min. consumption", drain);
            c.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|r| {
                r.spawn((
                    Text::new("Electricity:"),
                    TextFont { font: ctx.fonts.bold.clone(), font_size: 14.0, ..default() },
                    TextColor(KEY),
                ));
                r.spawn(Node { flex_grow: 1.0, flex_direction: FlexDirection::Column, ..default() }).with_children(
                    |b| {
                        let sat = sim
                            .0
                            .power
                            .electric_network_of
                            .get(&id)
                            .map_or(0.0, |n| sim.0.power.electric_networks[*n].satisfaction().to_f64_lossy());
                        progress_bar_live(b, sat, looks().production_bar_color, None);
                    },
                );
            });
        });
    }
    if let EntityState::Fluid(f) = &e.state
        && matches!(proto.data, EntityData::Generator { .. })
    {
        section(p, &mut |c| {
            ctx.tip_category(c, "Generates electricity");
            ctx.tip_kv(c, "Power output", power(f.last_power));
        });
    }
}

/// A number without trailing zeros ("2", "1.5").
fn trim(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}
