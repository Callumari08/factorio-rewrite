//! Screen UI: status text, the character inventory and crafting menu, entity windows and
//! the crafting queue. Panels are rebuilt only when what they show changes.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use factorio_sim::input::InputAction;
use factorio_sim::inventory::Inventory;
use factorio_sim::player::Character;
use factorio_sim::proto::{EntityData, ItemOrFluid, PrototypeDb, RecipeId};
use factorio_sim::world::EntityState;

use crate::controls::{MouseWorld, UiState};
use crate::sprites::Sprites;
use crate::{Cursor, Data, LOCAL_PLAYER, PendingInputs, Sim};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(Startup, setup)
            .add_systems(Update, (pointer_over_ui, buttons, hud, inventory_panel, entity_panel, queue_panel).chain());
    }
}

#[derive(Component)]
struct HudText;
#[derive(Component)]
struct StatusText;
#[derive(Component)]
struct InventoryRoot;
#[derive(Component)]
struct EntityRoot;
#[derive(Component)]
struct QueueRoot;

#[derive(Component, Clone)]
enum UiButton {
    Slot(usize),
    Craft(RecipeId),
    SetRecipe(Option<RecipeId>),
    GiveCursor,
    TakeAll,
    CancelCraft(u32),
}

const PANEL: Color = Color::srgb(0.12, 0.12, 0.12);
const SLOT: Color = Color::srgb(0.25, 0.25, 0.25);
const SLOT_SELECTED: Color = Color::srgb(0.65, 0.5, 0.15);

fn setup(mut commands: Commands) {
    commands.spawn((
        HudText,
        Text::new(""),
        TextFont { font_size: 14.0, ..default() },
        Node { position_type: PositionType::Absolute, bottom: Val::Px(84.0), left: Val::Px(8.0), ..default() },
    ));
    commands.spawn((
        StatusText,
        Text::new(""),
        TextFont { font_size: 15.0, ..default() },
        TextColor(Color::srgb(1.0, 0.85, 0.5)),
        Node { position_type: PositionType::Absolute, bottom: Val::Px(52.0), left: Val::Px(8.0), ..default() },
    ));
    commands.spawn((
        InventoryRoot,
        Interaction::default(),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            right: Val::Px(8.0),
            width: Val::Px(430.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(8.0)),
            row_gap: Val::Px(4.0),
            display: Display::None,
            ..default()
        },
        BackgroundColor(PANEL),
    ));
    commands.spawn((
        EntityRoot,
        Interaction::default(),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            right: Val::Px(450.0),
            width: Val::Px(330.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(8.0)),
            row_gap: Val::Px(4.0),
            display: Display::None,
            ..default()
        },
        BackgroundColor(PANEL),
    ));
    commands.spawn((
        QueueRoot,
        Interaction::default(),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            left: Val::Px(8.0),
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(2.0),
            ..default()
        },
    ));
}

fn pointer_over_ui(q: Query<&Interaction>, mut ui: ResMut<UiState>) {
    ui.pointer_over_ui = q.iter().any(|i| *i != Interaction::None);
}

fn character(sim: &Sim) -> Option<&Character> {
    sim.0.player(LOCAL_PLAYER).and_then(|p| p.character.as_ref())
}

/// Factorio's crafting menu order: item group, subgroup, then the recipe's own `order`
/// (falling back to its main product's), then name.
fn menu_order(data: &Data, db: &PrototypeDb, r: RecipeId) -> (String, String, String, String) {
    let d = &data.0;
    let rec = db.recipe(r);
    let raw = d.prototype("recipe", &rec.name);
    let product = rec.results.first().map(|p| match p.what {
        ItemOrFluid::Item(i) => {
            let item = db.item(i);
            d.prototype(&item.kind, &item.name)
        }
        ItemOrFluid::Fluid(f) => d.prototype("fluid", &db.fluid(f).name),
    });
    let get =
        |k: &str| raw.get(k).as_str().or_else(|| product.and_then(|p| p.get(k).as_str())).unwrap_or("").to_owned();
    let subgroup = get("subgroup");
    let sub = d.prototype("item-subgroup", &subgroup);
    let group = sub.get("group").as_str().unwrap_or("");
    let group_order = d.prototype("item-group", group).get("order").as_str().unwrap_or("").to_owned();
    (group_order, sub.get("order").as_str().unwrap_or("").to_owned(), get("order"), rec.name.clone())
}

fn hand_recipes(db: &PrototypeDb, c: &Character) -> Vec<RecipeId> {
    let cats = match &db.entity(c.proto).data {
        EntityData::Character { crafting_categories, .. } => crafting_categories.clone(),
        _ => Vec::new(),
    };
    db.recipe_ids()
        .filter(|r| {
            let rec = db.recipe(*r);
            !rec.hidden
                && cats.contains(&rec.category)
                && rec.ingredients.iter().all(|i| matches!(i.what, ItemOrFluid::Item(_)))
                && rec.results.iter().all(|p| matches!(p.what, ItemOrFluid::Item(_)))
        })
        .collect()
}

fn recipe_text(db: &PrototypeDb, r: RecipeId) -> String {
    let rec = db.recipe(r);
    let name = |w: &ItemOrFluid| match w {
        ItemOrFluid::Item(i) => db.item(*i).name.clone(),
        ItemOrFluid::Fluid(f) => db.fluid(*f).name.clone(),
    };
    let ing: Vec<String> = rec.ingredients.iter().map(|i| format!("{} {}", i.amount, name(&i.what))).collect();
    format!("{} ({}s): {}", rec.name, rec.energy_required, ing.join(", "))
}

fn buttons(
    q: Query<(&Interaction, &UiButton), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
    sim: Res<Sim>,
    mut ui: ResMut<UiState>,
    mut cursor: ResMut<Cursor>,
    mut pending: ResMut<PendingInputs>,
) {
    let db = sim.0.prototypes();
    let Some(c) = character(&sim) else { return };
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    for (interaction, button) in &q {
        match (interaction, button) {
            (Interaction::Hovered, UiButton::Craft(r)) => ui.status = recipe_text(db, *r),
            (Interaction::Hovered, UiButton::SetRecipe(Some(r))) => ui.status = recipe_text(db, *r),
            (Interaction::Hovered, UiButton::Slot(i)) => {
                if let Some(s) = c.inventory.slot(*i) {
                    ui.status = format!("{} x{}", db.item(s.item).name, s.count);
                }
            }
            (Interaction::Pressed, UiButton::Slot(i)) => {
                let item = c.inventory.slot(*i).map(|s| s.item);
                cursor.item = if cursor.item == item { None } else { item };
            }
            (Interaction::Pressed, UiButton::Craft(r)) => {
                pending.push(InputAction::Craft { recipe: *r, count: if shift { 5 } else { 1 } });
            }
            (Interaction::Pressed, UiButton::SetRecipe(r)) => {
                if let Some(position) = ui.opened {
                    pending.push(InputAction::SetRecipe { position, recipe: *r });
                }
            }
            (Interaction::Pressed, UiButton::GiveCursor) => {
                if let (Some(position), Some(item)) = (ui.opened, cursor.item) {
                    let count = if shift {
                        c.inventory.count(item)
                    } else {
                        db.item(item).stack_size.min(c.inventory.count(item))
                    };
                    pending.push(InputAction::TransferToEntity { position, item, count });
                }
            }
            (Interaction::Pressed, UiButton::TakeAll) => {
                if let Some(position) = ui.opened {
                    pending.push(InputAction::TakeFromEntity { position });
                }
            }
            (Interaction::Pressed, UiButton::CancelCraft(i)) => pending.push(InputAction::CancelCraft { index: *i }),
            _ => {}
        }
    }
}

fn hud(
    sim: Res<Sim>,
    mouse: Res<MouseWorld>,
    cursor: Res<Cursor>,
    ui: Res<UiState>,
    diagnostics: Res<DiagnosticsStore>,
    time: Res<Time<Real>>,
    mut texts: ParamSet<(Single<&mut Text, With<HudText>>, Single<&mut Text, With<StatusText>>)>,
    mut last: Local<(f32, u64, f64, u64)>,
) {
    let now = time.elapsed_secs();
    if now - last.0 >= 1.0 {
        last.2 = (sim.0.tick() - last.1) as f64 / (now - last.0) as f64;
        last.3 = sim.0.checksum();
        last.0 = now;
        last.1 = sim.0.tick();
    }
    let fps = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS).and_then(|d| d.smoothed()).unwrap_or(0.0);
    let db = sim.0.prototypes();
    let mut lines =
        vec![format!("tick {}  |  {:.0} UPS  {:.0} FPS  |  checksum {:016x}", sim.0.tick(), last.2, fps, last.3)];
    if let Some(c) = character(&sim) {
        let p = c.position();
        lines.push(format!("position {:.1}, {:.1}", p.x as f32 / 256.0, p.y as f32 / 256.0));
        if let Some(m) = &c.mining {
            lines.push(format!("mining… {:.0}%", m.progress.to_f64_lossy() * 100.0 / 60.0));
        }
        if let Some(job) = c.queue.first() {
            let rec = db.recipe(job.recipe);
            lines.push(format!(
                "crafting {} ({:.0}%)",
                rec.name,
                c.craft_progress.to_f64_lossy() / rec.ticks().to_f64_lossy() * 100.0
            ));
        }
    }
    if let Some(item) = cursor.item {
        lines.push(format!("holding {}  [R rotate, Q clear]", db.item(item).name));
    }
    for (i, n) in sim.0.power.electric_networks.iter().enumerate() {
        lines.push(format!(
            "power network {}: {:.0} kW used / {:.0} kW available ({:.0}%)",
            i + 1,
            n.production.to_f64_lossy() * 60.0 / 1000.0,
            n.capacity.to_f64_lossy() * 60.0 / 1000.0,
            n.satisfaction().to_f64_lossy() * 100.0
        ));
    }
    if let Some(id) = mouse.0.and_then(|p| sim.0.entity_at(p)) {
        lines.push(format!("hover: {}", db.entity(sim.0.entity(id).unwrap().proto).name));
    } else if let Some(r) = mouse.0.and_then(|p| sim.0.surface.resource(p.tile())) {
        lines.push(format!("hover: {} ({})", db.entity(r.proto).name, r.amount));
    }
    lines.push("WASD walk | LMB build/open | RMB mine | E inventory | F pick up | Q pipette | R rotate | wheel zoom | F1 sandbox kit".into());
    texts.p0().0 = lines.join("\n");
    texts.p1().0 = ui.status.clone();
}

fn slot_button(
    parent: &mut ChildSpawnerCommands,
    sprites: &mut Sprites,
    assets: &AssetServer,
    data: &Data,
    item: Option<&str>,
    count: Option<u32>,
    selected: bool,
    button: UiButton,
) {
    let mut e = parent.spawn((
        button,
        Button,
        Node {
            width: Val::Px(36.0),
            height: Val::Px(36.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(if selected { SLOT_SELECTED } else { SLOT }),
    ));
    e.with_children(|p| {
        if let Some(icon) = item.and_then(|i| sprites.item_icon(assets, data, i)) {
            let s = &icon.sprite;
            p.spawn((
                ImageNode {
                    image: icon.image.clone(),
                    rect: Some(Rect::new(s.x as f32, s.y as f32, (s.x + s.width) as f32, (s.y + s.height) as f32)),
                    ..default()
                },
                Node { width: Val::Px(32.0), height: Val::Px(32.0), ..default() },
            ));
        }
        if let Some(n) = count.filter(|n| *n > 1) {
            p.spawn((
                Text::new(n.to_string()),
                TextFont { font_size: 11.0, ..default() },
                Node { position_type: PositionType::Absolute, bottom: Val::Px(0.0), right: Val::Px(2.0), ..default() },
            ));
        }
    });
}

fn label(parent: &mut ChildSpawnerCommands, text: impl Into<String>, size: f32) {
    parent.spawn((Text::new(text.into()), TextFont { font_size: size, ..default() }));
}

fn grid(parent: &mut ChildSpawnerCommands, f: impl FnOnce(&mut ChildSpawnerCommands)) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            column_gap: Val::Px(2.0),
            row_gap: Val::Px(2.0),
            ..default()
        })
        .with_children(f);
}

fn inventory_slots(
    parent: &mut ChildSpawnerCommands,
    sprites: &mut Sprites,
    assets: &AssetServer,
    data: &Data,
    db: &PrototypeDb,
    inv: &Inventory,
    cursor: Option<factorio_sim::proto::ItemId>,
    clickable: bool,
) {
    grid(parent, |g| {
        for (i, s) in inv.slots().iter().enumerate() {
            let name = s.map(|s| db.item(s.item).name.clone());
            let selected = clickable && s.is_some_and(|s| Some(s.item) == cursor);
            let button = if clickable { UiButton::Slot(i) } else { UiButton::TakeAll };
            slot_button(g, sprites, assets, data, name.as_deref(), s.map(|s| s.count), selected, button);
        }
    });
}

fn inventory_panel(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    cursor: Res<Cursor>,
    ui: Res<UiState>,
    mut sprites: ResMut<Sprites>,
    mut root: Single<(Entity, &mut Node), With<InventoryRoot>>,
    mut last: Local<String>,
) {
    root.1.display = if ui.inventory_open { Display::Flex } else { Display::None };
    let Some(c) = character(&sim) else { return };
    let sig = format!("{:?}{:?}{}", c.inventory, cursor.item, ui.inventory_open);
    if *last == sig || !ui.inventory_open {
        return;
    }
    *last = sig;
    let db = sim.0.prototypes();
    let mut recipes = hand_recipes(db, c);
    recipes.sort_by_cached_key(|r| menu_order(&data, db, *r));
    let root = root.0;
    commands.entity(root).despawn_related::<Children>();
    commands.entity(root).with_children(|p| {
        label(p, "Character  (click an item to hold it)", 16.0);
        inventory_slots(p, &mut sprites, &assets, &data, db, &c.inventory, cursor.item, true);
        label(p, "Crafting  (click: 1, shift-click: 5)", 16.0);
        grid(p, |g| {
            for r in recipes {
                let rec = db.recipe(r);
                let main = rec.results.first().and_then(|p| match p.what {
                    ItemOrFluid::Item(i) => Some(db.item(i).name.clone()),
                    _ => None,
                });
                slot_button(g, &mut sprites, &assets, &data, main.as_deref(), None, false, UiButton::Craft(r));
            }
        });
    });
}

fn entity_panel(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    cursor: Res<Cursor>,
    ui: Res<UiState>,
    mut sprites: ResMut<Sprites>,
    mut root: Single<(Entity, &mut Node), With<EntityRoot>>,
    mut last: Local<String>,
) {
    let entity = ui.opened.and_then(|p| sim.0.entity_at(p)).and_then(|id| sim.0.entity(id).map(|e| (id, e)));
    root.1.display = if entity.is_some() && ui.inventory_open { Display::Flex } else { Display::None };
    let Some((id, e)) = entity else { return };
    let db = sim.0.prototypes();
    let proto = db.entity(e.proto);
    // Rebuild a few times a second at most; progress text changes every tick.
    let fluid = sim.0.power.fluid_network_of.keys().filter(|(eid, _)| *eid == id).count();
    let sig = format!("{:?}{:?}{:?}{}", e.state, sim.0.belts.get(id).map(|b| b.item_count()), cursor.item, fluid);
    let coarse = format!("{}{}", sig.len(), sim.0.tick() / 15);
    if *last == coarse {
        return;
    }
    *last = coarse;
    let root = root.0;
    commands.entity(root).despawn_related::<Children>();
    commands.entity(root).with_children(|p| {
        label(p, proto.name.clone(), 18.0);
        // Power: what the machine drew last tick against its maximum.
        let kw = |e: factorio_sim::Fixed| e.to_f64_lossy() * 60.0 / 1000.0;
        let max = factorio_sim::power::electric_buffer_capacity(proto);
        match proto.energy_source() {
            Some(factorio_sim::proto::EnergySource::Electric { .. }) => {
                let used = sim.0.power.last_consumption.get(&id).copied().unwrap_or_default();
                let net = sim.0.power.electric_network_of.get(&id).map(|n| &sim.0.power.electric_networks[*n]);
                let status = match net {
                    None => "not connected to a power network".to_owned(),
                    Some(n) => format!("network satisfaction {:.0}%", n.satisfaction().to_f64_lossy() * 100.0),
                };
                label(p, format!("Power: {:.1} kW of {:.1} kW max  ({status})", kw(used), kw(max)), 14.0);
            }
            Some(factorio_sim::proto::EnergySource::Burner { .. }) => {
                let usage = match &proto.data {
                    EntityData::MiningDrill { energy_usage, .. } | EntityData::CraftingMachine { energy_usage, .. } => {
                        Some(*energy_usage)
                    }
                    EntityData::Boiler { energy_consumption, .. } => Some(*energy_consumption),
                    _ => None,
                };
                if let Some(u) = usage {
                    label(p, format!("Burns fuel at {:.0} kW while working", kw(u)), 14.0);
                }
            }
            _ => {}
        }
        let fuel = |p: &mut ChildSpawnerCommands, sprites: &mut Sprites, energy: &factorio_sim::energy::EnergyState| {
            if let Some(b) = energy.burner() {
                label(p, format!("Fuel (burning: {:.0} kJ left)", b.remaining.to_f64_lossy() / 1000.0), 14.0);
                inventory_slots(p, sprites, &assets, &data, db, &b.fuel, None, false);
            } else if let factorio_sim::energy::EnergyState::Electric { buffer } = energy {
                label(p, format!("Electric buffer: {:.1} kJ", buffer.to_f64_lossy() / 1000.0), 14.0);
            }
        };
        match &e.state {
            EntityState::Container(inv) => inventory_slots(p, &mut sprites, &assets, &data, db, inv, None, false),
            EntityState::Drill(d) => {
                label(p, if d.working { "Working" } else { "Not working" }, 14.0);
                fuel(p, &mut sprites, &d.energy);
            }
            EntityState::Crafter(cr) => {
                let ticks = cr.recipe.map(|r| db.recipe(r).ticks().to_f64_lossy()).unwrap_or(1.0);
                label(
                    p,
                    format!(
                        "Recipe: {}  progress {:.0}%",
                        cr.recipe.map(|r| db.recipe(r).name.clone()).unwrap_or("none".into()),
                        cr.progress.to_f64_lossy() / ticks * 100.0
                    ),
                    14.0,
                );
                label(p, "Input", 14.0);
                inventory_slots(p, &mut sprites, &assets, &data, db, &cr.input, None, false);
                label(p, "Output", 14.0);
                inventory_slots(p, &mut sprites, &assets, &data, db, &cr.output, None, false);
                fuel(p, &mut sprites, &cr.energy);
                if !cr.furnace
                    && let EntityData::CraftingMachine { crafting_categories, .. } = &proto.data
                {
                    label(p, "Choose recipe", 14.0);
                    let mut ids: Vec<RecipeId> = db.recipe_ids().collect();
                    ids.sort_by_cached_key(|r| menu_order(&data, db, *r));
                    grid(p, |g| {
                        for r in ids {
                            let rec = db.recipe(r);
                            let items_only = rec.ingredients.iter().all(|i| matches!(i.what, ItemOrFluid::Item(_)))
                                && rec.results.iter().all(|x| matches!(x.what, ItemOrFluid::Item(_)));
                            if rec.hidden || !items_only || !crafting_categories.contains(&rec.category) {
                                continue;
                            }
                            let main = rec.results.first().and_then(|x| match x.what {
                                ItemOrFluid::Item(i) => Some(db.item(i).name.clone()),
                                _ => None,
                            });
                            slot_button(
                                g,
                                &mut sprites,
                                &assets,
                                &data,
                                main.as_deref(),
                                None,
                                cr.recipe == Some(r),
                                UiButton::SetRecipe(Some(r)),
                            );
                        }
                    });
                }
            }
            EntityState::Inserter(ins) => {
                label(
                    p,
                    format!("Holding: {}", ins.hand.map(|h| db.item(h.item).name.clone()).unwrap_or("nothing".into())),
                    14.0,
                );
                fuel(p, &mut sprites, &ins.energy);
            }
            EntityState::Fluid(f) => {
                for (i, b) in f.boxes.iter().enumerate() {
                    let fluid = b.fluid.map(|x| db.fluid(x).name.clone()).unwrap_or("empty".into());
                    label(
                        p,
                        format!(
                            "Fluid box {}: {} {:.1} at {:.0}°C",
                            i + 1,
                            fluid,
                            b.amount.to_f64_lossy(),
                            b.temperature.to_f64_lossy()
                        ),
                        14.0,
                    );
                }
                label(p, format!("Power: {:.0} kW", f.last_power.to_f64_lossy() * 60.0 / 1000.0), 14.0);
                fuel(p, &mut sprites, &f.energy);
            }
            EntityState::Belt => {
                label(p, format!("{} items on belt", sim.0.belts.get(id).map(|b| b.item_count()).unwrap_or(0)), 14.0);
            }
            _ => {}
        }
        p.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(6.0), ..default() }).with_children(
            |row| {
                for (text, button) in
                    [("Insert held item (shift: all)", UiButton::GiveCursor), ("Take all", UiButton::TakeAll)]
                {
                    row.spawn((
                        button,
                        Button,
                        Node { padding: UiRect::all(Val::Px(4.0)), ..default() },
                        BackgroundColor(SLOT),
                    ))
                    .with_children(|b| label(b, text, 13.0));
                }
            },
        );
    });
}

fn queue_panel(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    mut sprites: ResMut<Sprites>,
    root: Single<Entity, With<QueueRoot>>,
    mut last: Local<String>,
) {
    let Some(c) = character(&sim) else { return };
    let sig = format!("{:?}", c.queue.iter().map(|j| (j.recipe, j.count)).collect::<Vec<_>>());
    if *last == sig {
        return;
    }
    *last = sig;
    let db = sim.0.prototypes();
    commands.entity(*root).despawn_related::<Children>();
    commands.entity(*root).with_children(|p| {
        for (i, job) in c.queue.iter().enumerate() {
            let rec = db.recipe(job.recipe);
            let main = rec.results.first().and_then(|x| match x.what {
                ItemOrFluid::Item(i) => Some(db.item(i).name.clone()),
                _ => None,
            });
            slot_button(
                p,
                &mut sprites,
                &assets,
                &data,
                main.as_deref(),
                Some(job.count),
                false,
                UiButton::CancelCraft(i as u32),
            );
        }
    });
}
