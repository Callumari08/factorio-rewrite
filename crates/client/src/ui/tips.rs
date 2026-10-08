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
                ctx.tip_kv(p, "Time", format!("{} s", unit.time_ticks as u64 * count as u64 / 60));
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

/// The info panel for the entity under the mouse (shown at the side, below the side
/// menu, as in the game): name, status and the figures that matter for its type.
pub(super) fn entity_info(p: &mut ChildSpawnerCommands, ctx: &mut Ctx, names: &Names, sim: &Sim, id: EntityId) {
    let db = ctx.db;
    let Some(e) = sim.0.entity(id) else { return };
    let proto = db.entity(e.proto);
    ctx.tip_title(p, names.entity(e.proto).to_owned());
    let (st, _) = status(sim, id);
    if !st.is_empty() {
        ctx.tip_kv(p, "Status", st);
    }
    let power = |v: factorio_sim::Fixed| crate::chart::power_text(v.to_f64_lossy());
    let burner = |p: &mut ChildSpawnerCommands, ctx: &mut Ctx, energy: &factorio_sim::energy::EnergyState| {
        if let Some(b) = energy.burner() {
            ctx.tip_category(p, "Consumes Burnable fuel");
            if let Some(EnergySource::Burner { .. }) = proto.energy_source() {
                let usage = match &proto.data {
                    EntityData::MiningDrill { energy_usage, .. } | EntityData::CraftingMachine { energy_usage, .. } => {
                        Some(*energy_usage)
                    }
                    EntityData::Boiler { energy_consumption, .. } => Some(*energy_consumption),
                    _ => None,
                };
                if let Some(u) = usage {
                    ctx.tip_kv(p, "Max consumption", power(u));
                }
            }
            for s in b.fuel.slots().iter().flatten() {
                ctx.tip_amount(p, Some(s.item), s.count.to_string(), names.item(s.item).to_owned());
            }
        }
    };
    match &e.state {
        EntityState::Crafter(c) => {
            if let Some(r) = c.recipe {
                ctx.tip_kv(p, "Recipe", names.recipe(r).to_owned());
            }
            burner(p, ctx, &c.energy);
        }
        EntityState::Drill(d) => burner(p, ctx, &d.energy),
        EntityState::Inserter(i) => burner(p, ctx, &i.energy),
        EntityState::Lab(l) => burner(p, ctx, &l.energy),
        EntityState::Fluid(f) => {
            for b in &f.boxes {
                if let Some(fluid) = b.fluid {
                    let name = db.fluid(fluid).name.clone();
                    ctx.tip_kv(p, &name, format!("{:.1}", b.amount.to_f64_lossy()));
                    ctx.tip_kv(p, "Temperature", format!("{:.2} °C", b.temperature.to_f64_lossy()));
                }
            }
            burner(p, ctx, &f.energy);
            if matches!(proto.data, EntityData::Generator { .. }) {
                ctx.tip_category(p, "Generates electricity");
                ctx.tip_kv(p, "Power output", power(f.last_power));
            }
        }
        EntityState::Container(inv) => {
            let used = inv.slots().iter().filter(|s| s.is_some()).count();
            ctx.tip_kv(p, "Inventory", format!("{used}/{}", inv.len()));
        }
        _ => {}
    }
    if let Some(EnergySource::Electric { .. }) = proto.energy_source() {
        ctx.tip_category(p, "Consumes electricity");
        let used = sim.0.power.last_consumption.get(&id).copied().unwrap_or_default();
        ctx.tip_kv(p, "Consumption", power(used));
        ctx.tip_kv(p, "Max consumption", power(factorio_sim::power::electric_buffer_capacity(proto)));
    }
}
