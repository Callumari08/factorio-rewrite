//! Screen UI modelled on Factorio's: quickbar, character window (inventory + crafting with
//! item-group tabs), entity windows with clickable slots, tooltips, the held item on the
//! mouse, and a status/HUD line. All item movement goes through simulation inputs.

use std::collections::HashMap;

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use factorio_sim::input::{EntityInventory, InputAction, MouseButton as SimButton, SlotRef};
use factorio_sim::inventory::Inventory;
use factorio_sim::player::{Character, QUICKBAR_SLOTS, max_craftable};
use factorio_sim::proto::{EnergySource, EntityData, ItemId, ItemOrFluid, PrototypeDb, RecipeId};
use factorio_sim::research::Research;
use factorio_sim::world::{EntityId, EntityState};

use crate::controls::{MouseWorld, UiState};
use crate::gui_skin::looks;
use factorio_sim::proto::TechId;

mod research;
use crate::sprites::Sprites;
use crate::{Data, LOCAL_PLAYER, PendingInputs, Sim};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(Startup, (load_names, setup, research::setup).chain())
            .add_systems(
                Update,
                (
                    pointer_over_ui,
                    clicks,
                    hud,
                    quickbar,
                    queue,
                    window,
                    research::scroll,
                    research::window,
                    research::hud,
                    hover_highlight,
                    tooltip,
                    cursor_icon,
                    selection_box,
                )
                    .chain(),
            );
    }
}

// ----- style (Factorio's palette) -----

const FRAME: Color = Color::srgb(0.192, 0.188, 0.192);
const INNER: Color = Color::srgb(0.141, 0.137, 0.141);
const SLOT: Color = Color::srgb(0.239, 0.235, 0.239);
/// Marks an inventory slot (the game's darker `inventory_slot` style).
const INV: Color = Color::srgb(0.239, 0.235, 0.240);
const SLOT_HOVER: Color = Color::srgb(0.36, 0.34, 0.30);
const SLOT_RED: Color = Color::srgb(0.38, 0.17, 0.15);
const TAB_SELECTED: Color = Color::srgb(0.55, 0.42, 0.18);
const HEADING: Color = Color::srgb(1.0, 0.902, 0.753);
const TEXT: Color = Color::srgb(0.9, 0.9, 0.9);
const PROGRESS: Color = Color::srgb(0.38, 0.72, 0.29);
const SLOT_PX: f32 = 40.0;
/// An `inside_shallow_frame_with_padding` panel around a 10-slot table.
const PANEL_W: f32 = SLOT_PX * 10.0 + 24.0;
/// The game's `entity_button_frame`: 10 slots wide, 4 slots (less spacing) high.
const PREVIEW_W: f32 = SLOT_PX * 10.0;
const PREVIEW_H: f32 = SLOT_PX * 4.0 - 8.0;

#[derive(Resource)]
#[allow(dead_code)]
pub struct Fonts {
    pub regular: Handle<Font>,
    pub semibold: Handle<Font>,
    pub bold: Handle<Font>,
}

/// Localised display names, resolved once at startup.
#[derive(Resource, Default)]
pub struct Names {
    items: HashMap<ItemId, String>,
    recipes: HashMap<RecipeId, String>,
    entities: HashMap<factorio_sim::proto::EntityProtoId, String>,
    groups: HashMap<String, String>,
    /// Technology names and whether the level is shown after them.
    techs: HashMap<TechId, (String, bool)>,
    tech_descriptions: HashMap<TechId, String>,
    /// `modifier-description` strings by key, e.g. `bullet-damage-bonus`.
    modifiers: HashMap<String, String>,
    /// Crafting menu: groups (name, icon item group) with rows of recipes per subgroup.
    menu: Vec<MenuGroup>,
}

struct MenuGroup {
    name: String,
    rows: Vec<Vec<RecipeId>>,
}

impl Names {
    pub fn item(&self, i: ItemId) -> &str {
        self.items.get(&i).map(String::as_str).unwrap_or("?")
    }
    pub fn recipe(&self, r: RecipeId) -> &str {
        self.recipes.get(&r).map(String::as_str).unwrap_or("?")
    }
    pub fn entity(&self, e: factorio_sim::proto::EntityProtoId) -> &str {
        self.entities.get(&e).map(String::as_str).unwrap_or("?")
    }
}

#[derive(Component)]
struct HudText;
#[derive(Component)]
struct StatusText;
#[derive(Component)]
struct QuickbarRoot;
#[derive(Component)]
struct WindowRoot;
#[derive(Component)]
struct TooltipRoot;
#[derive(Component)]
struct CursorIcon;
#[derive(Component)]
struct QueueRoot;
/// A slot's normal background, restored when the mouse leaves it.
#[derive(Component)]
struct Base(Color);

/// What a clickable UI element does.
#[derive(Component, Clone, Debug)]
enum UiButton {
    Slot(SlotRef),
    Craft(RecipeId),
    Quickbar(usize),
    Tab(usize),
    SetRecipe(RecipeId),
    ChangeRecipe,
    ChartRange(usize),
    QueueCancel(u32),
    SelectTech(TechId),
    QueueTech(TechId),
    DequeueTech(TechId),
    OpenTech,
    CloseWindow,
}

/// What the tooltip should describe when this element is hovered.
#[derive(Component, Clone, Debug)]
enum Tip {
    Item(ItemId),
    Recipe(RecipeId),
    Tech(TechId),
    Text(String),
}

/// Client-only UI state.
#[derive(Resource, Default)]
struct Local_ {
    tab: usize,
    choosing_recipe: bool,
}

fn load_names(mut commands: Commands, sim: Res<Sim>, data: Res<Data>, assets: Res<AssetServer>) {
    let font = |f: &str| assets.load(format!("factorio://data/core/fonts/{f}"));
    commands.insert_resource(Fonts {
        regular: font("TitilliumWeb-Regular.ttf"),
        semibold: font("TitilliumWeb-SemiBold.ttf"),
        bold: font("TitilliumWeb-Bold.ttf"),
    });
    commands.init_resource::<Local_>();

    let d = &data.0;
    let lang = std::env::var("FACTORIO_REWRITE_LOCALE").unwrap_or_else(|_| "en".into());
    let locale = factorio_data::locale::Locale::load(d, &lang);
    let db = sim.0.prototypes();
    let mut names = Names::default();
    for i in db.item_ids() {
        names.items.insert(i, locale.item_name(d, &db.item(i).name));
    }
    for r in db.recipe_ids() {
        names.recipes.insert(r, locale.recipe_name(d, &db.recipe(r).name));
    }
    for e in db.entity_ids() {
        names.entities.insert(e, locale.entity_name(d, &db.entity(e).name));
    }

    for t in db.technology_ids() {
        let name = &db.technology(t).name;
        names.techs.insert(t, locale.technology_name(d, name));
        if let Some(desc) = locale.technology_description(name) {
            names.tech_descriptions.insert(t, desc);
        }
        for e in &db.technology(t).effects {
            if let factorio_sim::proto::TechEffect::Modifier { kind, qualifier, .. } = e {
                for key in [
                    kind.clone(),
                    format!("{qualifier}-damage-bonus"),
                    format!("{qualifier}-shooting-speed-bonus"),
                    format!("{qualifier}-attack-bonus"),
                ] {
                    if let Some(text) = locale.get(&format!("modifier-description.{key}")) {
                        names.modifiers.insert(key, text.to_owned());
                    }
                }
            }
        }
    }

    // Crafting menu layout: item groups in order, a row per subgroup.
    let mut entries: Vec<(String, String, String, String, String, RecipeId)> = Vec::new();
    for r in db.recipe_ids() {
        let rec = db.recipe(r);
        if rec.hidden {
            continue;
        }
        let raw = d.prototype("recipe", &rec.name);
        let product = rec.results.first().map(|p| match p.what {
            ItemOrFluid::Item(i) => d.prototype(&db.item(i).kind, &db.item(i).name),
            ItemOrFluid::Fluid(f) => d.prototype("fluid", &db.fluid(f).name),
        });
        let get =
            |k: &str| raw.get(k).as_str().or_else(|| product.and_then(|p| p.get(k).as_str())).unwrap_or("").to_owned();
        let subgroup = get("subgroup");
        let sub = d.prototype("item-subgroup", &subgroup);
        let group = sub.get("group").as_str().unwrap_or("other").to_owned();
        let group_order = d.prototype("item-group", &group).get("order").as_str().unwrap_or("").to_owned();
        let sub_order = sub.get("order").as_str().unwrap_or("").to_owned();
        entries.push((group_order, group, sub_order + &subgroup, get("order"), rec.name.clone(), r));
    }
    entries.sort();
    for (_, group, sub, _, _, r) in entries {
        if names.menu.last().is_none_or(|g| g.name != group) {
            names.groups.insert(group.clone(), locale.item_group_name(&group));
            names.menu.push(MenuGroup { name: group.clone(), rows: Vec::new() });
        }
        let g = names.menu.last_mut().unwrap();
        let new_row = g.rows.last().is_none_or(|row: &Vec<RecipeId>| {
            let last = *row.last().unwrap();
            sub_key(db, d, last) != sub
        });
        if new_row {
            g.rows.push(Vec::new());
        }
        g.rows.last_mut().unwrap().push(r);
    }
    commands.insert_resource(names);
}

fn sub_key(db: &PrototypeDb, d: &factorio_data::GameData, r: RecipeId) -> String {
    let rec = db.recipe(r);
    let raw = d.prototype("recipe", &rec.name);
    let product = rec.results.first().map(|p| match p.what {
        ItemOrFluid::Item(i) => d.prototype(&db.item(i).kind, &db.item(i).name),
        ItemOrFluid::Fluid(f) => d.prototype("fluid", &db.fluid(f).name),
    });
    let subgroup = raw
        .get("subgroup")
        .as_str()
        .or_else(|| product.and_then(|p| p.get("subgroup").as_str()))
        .unwrap_or("")
        .to_owned();
    let order = d.prototype("item-subgroup", &subgroup).get("order").as_str().unwrap_or("").to_owned();
    order + &subgroup
}

fn setup(mut commands: Commands, fonts: Res<Fonts>) {
    let text = |size: f32| TextFont { font: fonts.regular.clone(), font_size: size, ..default() };
    commands.spawn((
        HudText,
        Text::new(""),
        text(14.0),
        TextColor(TEXT),
        Node { position_type: PositionType::Absolute, top: Val::Px(6.0), left: Val::Px(8.0), ..default() },
    ));
    commands.spawn((
        StatusText,
        Text::new(""),
        text(15.0),
        TextColor(HEADING),
        Node { position_type: PositionType::Absolute, bottom: Val::Px(112.0), left: Val::Percent(30.0), ..default() },
    ));
    commands.spawn((
        QuickbarRoot,
        Interaction::default(),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(6.0),
            left: Val::Percent(50.0),
            margin: UiRect::left(Val::Px(-(SLOT_PX * 10.0 + 24.0) / 2.0)),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(6.0)),
            row_gap: Val::Px(2.0),
            ..default()
        },
        crate::gui_skin::node_image(&looks().frame),
    ));
    commands.spawn((
        WindowRoot,
        Interaction::default(),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(60.0),
            left: Val::Percent(50.0),
            margin: UiRect::left(Val::Px(-(PANEL_W + 6.0 + 8.0))),
            flex_direction: FlexDirection::Column,
            // The game's `frame` style padding.
            padding: UiRect { left: Val::Px(8.0), right: Val::Px(8.0), top: Val::Px(4.0), bottom: Val::Px(8.0) },
            display: Display::None,
            ..default()
        },
        crate::gui_skin::node_image(&looks().frame),
    ));
    commands.spawn((
        QueueRoot,
        Interaction::default(),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(6.0),
            left: Val::Px(8.0),
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            max_width: Val::Px(SLOT_PX * 8.0),
            ..default()
        },
    ));
    commands.spawn((
        TooltipRoot,
        Node {
            position_type: PositionType::Absolute,
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(8.0)),
            row_gap: Val::Px(3.0),
            display: Display::None,
            max_width: Val::Px(360.0),
            ..default()
        },
        crate::gui_skin::node_image(&looks().tooltip),
        GlobalZIndex(10),
    ));
    commands
        .spawn((
            CursorIcon,
            ImageNode::default(),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(32.0),
                height: Val::Px(32.0),
                display: Display::None,
                ..default()
            },
            GlobalZIndex(20),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(""),
                text(13.0),
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(-4.0),
                    bottom: Val::Px(-6.0),
                    ..default()
                },
            ));
        });
}

fn character(sim: &Sim) -> Option<&Character> {
    sim.0.player(LOCAL_PLAYER).and_then(|p| p.character.as_ref())
}

fn opened(sim: &Sim) -> Option<EntityId> {
    sim.0.player(LOCAL_PLAYER).and_then(|p| p.opened)
}

fn pointer_over_ui(q: Query<&Interaction>, mut ui: ResMut<UiState>) {
    ui.pointer_over_ui = q.iter().any(|i| *i != Interaction::None);
}

/// Mouse clicks on UI elements, with Factorio's button and modifier meanings.
fn clicks(
    q: Query<(&Interaction, &UiButton)>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    sim: Res<Sim>,
    mut pending: ResMut<PendingInputs>,
    mut local: ResMut<Local_>,
    mut chart: ResMut<crate::chart::Chart>,
    mut tech: ResMut<research::TechUi>,
    mut ui: ResMut<UiState>,
    mut gui_clicks: ResMut<crate::sound::GuiClicks>,
) {
    let button = if mouse.just_pressed(MouseButton::Left) {
        SimButton::Left
    } else if mouse.just_pressed(MouseButton::Right) {
        SimButton::Right
    } else {
        return;
    };
    let Some((_, target)) = q.iter().find(|(i, _)| **i != Interaction::None) else { return };
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if !matches!(target, UiButton::Slot(_) | UiButton::Craft(_)) {
        gui_clicks.0 += 1;
    }
    let player = sim.0.player(LOCAL_PLAYER);
    let held = character(&sim).and_then(|c| c.cursor).map(|c| c.item);
    match (target.clone(), button) {
        (UiButton::Slot(slot), b) => pending.push(InputAction::ClickSlot { slot, button: b, shift, ctrl }),
        (UiButton::Craft(recipe), SimButton::Left) => {
            pending.push(InputAction::Craft { recipe, count: if shift { u32::MAX } else { 1 } })
        }
        (UiButton::Craft(recipe), SimButton::Right) => pending.push(InputAction::Craft { recipe, count: 5 }),
        (UiButton::Quickbar(i), SimButton::Left) => match (held, player.and_then(|p| p.quickbar[i])) {
            (Some(item), _) => pending.push(InputAction::SetQuickbar { index: i as u8, item: Some(item) }),
            (None, Some(item)) => pending.push(InputAction::PickItem(item)),
            _ => {}
        },
        (UiButton::Quickbar(i), SimButton::Right) => {
            pending.push(InputAction::SetQuickbar { index: i as u8, item: None })
        }
        (UiButton::Tab(t), _) => local.tab = t,
        (UiButton::SetRecipe(r), _) => {
            if let Some(id) = opened(&sim) {
                let position = sim.0.entity(id).unwrap().position;
                pending.push(InputAction::SetRecipe { position, recipe: Some(r) });
                local.choosing_recipe = false;
            }
        }
        (UiButton::ChangeRecipe, _) => local.choosing_recipe = !local.choosing_recipe,
        (UiButton::ChartRange(r), _) => chart.range = r,
        (UiButton::QueueCancel(i), _) => pending.push(InputAction::CancelCraft { index: i }),
        (UiButton::SelectTech(t), SimButton::Left) => tech.selected = Some(t),
        // Right click on a queued technology removes it, as in the game's queue.
        (UiButton::SelectTech(t), SimButton::Right) => pending.push(InputAction::DequeueResearch(t)),
        (UiButton::QueueTech(t), _) => pending.push(InputAction::QueueResearch { tech: t, front: shift }),
        (UiButton::DequeueTech(t), _) => pending.push(InputAction::DequeueResearch(t)),
        (UiButton::CloseWindow, _) => {
            ui.inventory_open = false;
            pending.push(InputAction::OpenEntity(None));
        }
        (UiButton::OpenTech, _) => {
            ui.tech_open = true;
            tech.selected = sim.0.research().current();
        }
    }
}

fn hud(
    sim: Res<Sim>,
    mouse: Res<MouseWorld>,
    names: Res<Names>,
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
    let mut lines = vec![format!("{:.0} UPS  {:.0} FPS  tick {}  checksum {:016x}", last.2, fps, sim.0.tick(), last.3)];
    if let Some(c) = character(&sim) {
        let p = c.position();
        lines.push(format!("x {:.1}  y {:.1}", p.x as f32 / 256.0, p.y as f32 / 256.0));
    }
    if let Some(id) = mouse.0.and_then(|p| sim.0.entity_at(p)) {
        lines.push(names.entity(sim.0.entity(id).unwrap().proto).to_owned());
    } else if let Some(r) = mouse.0.and_then(|p| sim.0.surface.resource(p.tile())) {
        lines.push(format!("{}  ({} remaining)", names.entity(r.proto), r.amount));
    }
    lines.push(
        "E inventory  |  T technologies  |  Q clear/pick  |  R rotate  |  F pick up  |  Ctrl+click fast transfer  |  F1 all items  |  F2 cheat mode  |  F3 research all"
            .into(),
    );
    texts.p0().0 = lines.join("\n");
    // Mining and crafting progress, like the game's bars above the quickbar.
    let mut status = ui.status.clone();
    if let Some(c) = character(&sim) {
        if let Some(m) = &c.mining {
            let ticks = match &m.target {
                factorio_sim::player::MiningTarget::Entity(id) => {
                    sim.0.entity(*id).and_then(|e| db.entity(e.proto).minable.as_ref())
                }
                factorio_sim::player::MiningTarget::Resource(t) => {
                    sim.0.surface.resource(*t).and_then(|r| db.entity(r.proto).minable.as_ref())
                }
            }
            .map(|m| m.mining_ticks.to_f64_lossy())
            .unwrap_or(1.0);
            status = format!("Mining  {:.0}%", m.progress.to_f64_lossy() / ticks * 100.0);
        } else if let Some(job) = c.queue.first() {
            let t = db.recipe(job.recipe).ticks().to_f64_lossy();
            status =
                format!("Crafting {}  {:.0}%", names.recipe(job.recipe), c.craft_progress.to_f64_lossy() / t * 100.0);
        }
    }
    texts.p1().0 = status;
}

// ----- building blocks -----

struct Ctx<'a> {
    sprites: &'a mut Sprites,
    assets: &'a AssetServer,
    data: &'a Data,
    fonts: &'a Fonts,
    db: &'a PrototypeDb,
    research: &'a Research,
}

impl Ctx<'_> {
    fn text(&self, p: &mut ChildSpawnerCommands, s: impl Into<String>, size: f32, color: Color) {
        p.spawn((
            Text::new(s.into()),
            TextFont { font: self.fonts.regular.clone(), font_size: size, ..default() },
            TextColor(color),
        ));
    }

    fn heading(&self, p: &mut ChildSpawnerCommands, s: impl Into<String>) {
        p.spawn((
            Text::new(s.into()),
            TextFont { font: self.fonts.bold.clone(), font_size: 18.0, ..default() },
            TextColor(HEADING),
        ));
    }

    /// A panel's caption (the "Character" over the inventory).
    fn subheading(&self, p: &mut ChildSpawnerCommands, s: impl Into<String>) {
        p.spawn((
            Text::new(s.into()),
            TextFont { font: self.fonts.semibold.clone(), font_size: 15.0, ..default() },
            TextColor(Color::WHITE),
        ));
    }

    /// A `utility-sprites` picture drawn `size` px square.
    fn utility(&mut self, p: &mut ChildSpawnerCommands, name: &str, size: f32) {
        let d = self.data.0.clone();
        let n = name.to_owned();
        let Some(s) = self
            .sprites
            .get(self.assets, self.data, &format!("utility:{name}"), || factorio_data::sprite::utility_sprite(&d, &n))
        else {
            return;
        };
        let r = &s.sprite;
        p.spawn((
            ImageNode {
                image: s.image.clone(),
                rect: Some(Rect::new(r.x as f32, r.y as f32, (r.x + r.width) as f32, (r.y + r.height) as f32)),
                ..default()
            },
            Node { width: Val::Px(size), height: Val::Px(size), ..default() },
            Pickable::IGNORE,
        ));
    }

    fn icon(&mut self, p: &mut ChildSpawnerCommands, item: ItemId, size: f32) {
        let name = self.db.item(item).name.clone();
        if let Some(icon) = self.sprites.item_icon(self.assets, self.data, &name) {
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

    /// An inventory-style slot with an optional item, count and click/tooltip behaviour.
    fn slot(
        &mut self,
        p: &mut ChildSpawnerCommands,
        item: Option<ItemId>,
        count: Option<u32>,
        bg: Color,
        button: Option<UiButton>,
        tip: Option<Tip>,
    ) {
        // The game's slot styles: inventory, plain, red (missing) and yellow (selected).
        let look = if bg == SLOT_RED {
            &looks().red_slot
        } else if bg == TAB_SELECTED {
            &looks().yellow_slot
        } else if bg == INV {
            &looks().inventory_slot
        } else {
            &looks().slot
        };
        let mut e = p.spawn((
            Node {
                width: Val::Px(SLOT_PX),
                height: Val::Px(SLOT_PX),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            crate::gui_skin::node_image(&look.default),
            look.clone(),
        ));
        if let Some(b) = button {
            e.insert((b, Button));
        } else {
            e.insert(Interaction::default());
        }
        if let Some(t) = tip.or(item.map(Tip::Item)) {
            e.insert(t);
        }
        let font = self.fonts.bold.clone();
        e.with_children(|c| {
            if let Some(i) = item {
                self.icon(c, i, 32.0);
            }
            if let Some(n) = count.filter(|n| *n > 1) {
                // The game's `count-font`: bold 13 with a dark outline.
                c.spawn((
                    Text::new(n.to_string()),
                    TextFont { font: font.clone(), font_size: 13.0, ..default() },
                    TextColor(Color::WHITE),
                    TextShadow { offset: Vec2::new(1.0, 1.0), color: Color::BLACK },
                    Node {
                        position_type: PositionType::Absolute,
                        bottom: Val::Px(0.0),
                        right: Val::Px(2.0),
                        ..default()
                    },
                ));
            }
        });
    }

    fn inventory(
        &mut self,
        p: &mut ChildSpawnerCommands,
        inv: &Inventory,
        columns: usize,
        make: impl Fn(usize) -> SlotRef,
    ) {
        grid(p, columns, |g| {
            for (i, s) in inv.slots().iter().enumerate() {
                self.slot(g, s.map(|s| s.item), s.map(|s| s.count), INV, Some(UiButton::Slot(make(i))), None);
            }
        });
    }
}

fn grid(p: &mut ChildSpawnerCommands, columns: usize, f: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn(Node {
        display: Display::Grid,
        grid_template_columns: RepeatedGridTrack::px(columns as u16, SLOT_PX),
        column_gap: Val::Px(0.0),
        row_gap: Val::Px(0.0),
        ..default()
    })
    .with_children(f);
}

/// A frame's title bar: the title, the striped draggable filler and the close button.
fn frame_header(w: &mut ChildSpawnerCommands, ctx: &mut Ctx, title: &str) {
    let l = looks();
    w.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        // Measured from the game: the panels start 40 px below the frame's top edge.
        height: Val::Px(36.0),
        padding: UiRect::bottom(Val::Px(4.0)),
        ..default()
    })
    .with_children(|h| {
        h.spawn((
            Text::new(title),
            TextFont { font: ctx.fonts.bold.clone(), font_size: 18.0, ..default() },
            TextColor(l.title_color),
        ));
        let mut filler =
            h.spawn(Node { flex_grow: 1.0, height: Val::Px(24.0), margin: UiRect::left(Val::Px(4.0)), ..default() });
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
        h.spawn((
            UiButton::CloseWindow,
            Button,
            Tip::Text("Close".into()),
            Node {
                width: Val::Px(24.0),
                height: Val::Px(24.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            crate::gui_skin::node_image(&l.frame_button.default),
            l.frame_button.clone(),
        ))
        .with_children(|b| ctx.utility(b, "close", 16.0));
    });
}

/// The checkered backdrop of entity previews: tile-sized squares two shades apart,
/// lighter towards the bottom (measured from the game).
fn preview_background() -> Image {
    let (w, h) = (PREVIEW_W as u32, PREVIEW_H as u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let base = 37.0 + 36.0 * y as f32 / (h - 1) as f32;
            // Squares offset to match the game's (the preview is centred on the entity).
            let odd = ((x + 16) / 32 + (y + 22) / 32) % 2 == 1;
            let v = (base + if odd { 12.0 } else { 0.0 }) as u8;
            px.extend_from_slice(&[v, v, v, 255]);
        }
    }
    Image::new(
        bevy::render::render_resource::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        px,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    )
}

/// The game's `entity_button_frame`: the entity drawn at normal zoom over a checkered
/// backdrop, in a deep frame.
fn entity_preview(p: &mut ChildSpawnerCommands, ctx: &mut Ctx, sim: &Sim, id: EntityId, checker: &Handle<Image>) {
    let look = crate::render::entity_look(sim, ctx.data, ctx.sprites, ctx.assets, id);
    p.spawn(Node { width: Val::Px(PREVIEW_W), height: Val::Px(PREVIEW_H), overflow: Overflow::clip(), ..default() })
        .with_children(|v| {
            crate::gui_skin::backdrop(v, &looks().deep_in_shallow);
            // Inside the frame's border.
            let inset = Val::Px(looks().deep_in_shallow.border * looks().scale);
            v.spawn((
                ImageNode::new(checker.clone()),
                Node {
                    position_type: PositionType::Absolute,
                    left: inset,
                    top: inset,
                    right: inset,
                    bottom: inset,
                    ..default()
                },
                ZIndex(-1),
                Pickable::IGNORE,
            ));
            let centre = Vec2::new(PREVIEW_W, PREVIEW_H) / 2.0;
            for (l, kind) in look.map(|(_, layers)| layers).unwrap_or_default() {
                let s = &l.sprite;
                let size = Vec2::new(s.width as f32, s.height as f32) * s.scale as f32;
                let at = centre + Vec2::new(s.shift.0 as f32, s.shift.1 as f32) * 32.0 - size / 2.0;
                let color = match kind {
                    factorio_data::sprite::LayerKind::Shadow => Color::srgba(0.0, 0.0, 0.0, 0.55),
                    factorio_data::sprite::LayerKind::Normal => Color::WHITE,
                    factorio_data::sprite::LayerKind::Tinted([r, g, b, a]) => Color::srgba_u8(r, g, b, a),
                };
                v.spawn((
                    ImageNode {
                        image: l.image.clone(),
                        rect: Some(Rect::new(s.x as f32, s.y as f32, (s.x + s.width) as f32, (s.y + s.height) as f32)),
                        color,
                        ..default()
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(at.x),
                        top: Val::Px(at.y),
                        width: Val::Px(size.x),
                        height: Val::Px(size.y),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
        });
}

/// The game's `production_progressbar`: a 24 px bar with the percentage inside it.
fn production_bar(p: &mut ChildSpawnerCommands, ctx: &Ctx, fraction: f64) {
    let l = looks();
    p.spawn((
        Node {
            flex_grow: 1.0,
            height: Val::Px(24.0),
            justify_content: JustifyContent::FlexEnd,
            align_items: AlignItems::Center,
            padding: UiRect::right(Val::Px(8.0)),
            ..default()
        },
        crate::gui_skin::node_image(&l.bar_background),
    ))
    .with_children(|b| {
        let mut bar = crate::gui_skin::node_image(&l.bar);
        bar.color = l.production_bar_color;
        b.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent((fraction * 100.0) as f32),
                height: Val::Percent(100.0),
                ..default()
            },
            bar,
        ));
        b.spawn((
            Text::new(format!("{:.0}%", fraction * 100.0)),
            TextFont { font: ctx.fonts.regular.clone(), font_size: 14.0, ..default() },
            TextColor(if fraction > 0.9 { Color::BLACK } else { Color::WHITE }),
        ));
    });
}

/// The game's `line` style: a horizontal separator.
fn separator(p: &mut ChildSpawnerCommands) {
    let mut e = p.spawn(Node { width: Val::Percent(100.0), height: Val::Px(4.0), ..default() });
    if let Some((image, rect)) = &looks().line {
        e.insert(ImageNode {
            image: image.clone(),
            rect: Some(*rect),
            image_mode: bevy::ui::widget::NodeImageMode::Tiled {
                tile_x: true,
                tile_y: false,
                stretch_value: looks().scale,
            },
            ..default()
        });
    }
}

/// The game's `entity_frame`: an inside shallow frame with 12 px padding (8 on top).
fn panel(p: &mut ChildSpawnerCommands, width: f32, f: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn(Node {
        width: Val::Px(width),
        flex_direction: FlexDirection::Column,
        padding: UiRect { left: Val::Px(12.0), right: Val::Px(12.0), top: Val::Px(8.0), bottom: Val::Px(12.0) },
        row_gap: Val::Px(8.0),
        ..default()
    })
    .with_children(|c| {
        crate::gui_skin::backdrop(c, &looks().shallow);
        f(c);
    });
}

/// The game's `progressbar` style: a sliced background and a bar tinted `color`.
fn progress_bar(p: &mut ChildSpawnerCommands, fraction: f64, color: Color) {
    let l = looks();
    p.spawn((
        Node { width: Val::Percent(100.0), height: Val::Px(8.0), ..default() },
        crate::gui_skin::node_image(&l.bar_background),
    ))
    .with_children(|b| {
        let mut bar = crate::gui_skin::node_image(&l.bar);
        bar.color = color;
        b.spawn((
            Node {
                width: Val::Percent((fraction.clamp(0.0, 1.0) * 100.0) as f32),
                height: Val::Percent(100.0),
                ..default()
            },
            bar,
        ));
    });
}

// ----- quickbar -----

fn quickbar(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    fonts: Res<Fonts>,
    mut sprites: ResMut<Sprites>,
    root: Single<Entity, With<QuickbarRoot>>,
    mut last: Local<String>,
) {
    let Some(p) = sim.0.player(LOCAL_PLAYER) else { return };
    let Some(c) = p.character.as_ref() else { return };
    let counts: Vec<u32> = p
        .quickbar
        .iter()
        .map(|q| q.map_or(0, |i| c.inventory.count(i) + c.cursor.filter(|s| s.item == i).map_or(0, |s| s.count)))
        .collect();
    let sig = format!("{:?}{:?}", p.quickbar, counts);
    if *last == sig {
        return;
    }
    *last = sig;
    let db = sim.0.prototypes();
    let mut ctx =
        Ctx { sprites: &mut sprites, assets: &assets, data: &data, fonts: &fonts, db, research: sim.0.research() };
    commands.entity(*root).despawn_related::<Children>();
    commands.entity(*root).with_children(|r| {
        for row in 0..QUICKBAR_SLOTS / 10 {
            r.spawn(Node { flex_direction: FlexDirection::Row, ..default() }).with_children(|rr| {
                for k in 0..10 {
                    let i = row * 10 + k;
                    let item = p.quickbar[i];
                    let tip = item.map(Tip::Item).or(Some(Tip::Text(format!(
                        "Quickbar slot {}: click while holding an item to assign; right click clears",
                        (k + 1) % 10
                    ))));
                    let bg = if item.is_some() && counts[i] == 0 { SLOT_RED } else { SLOT };
                    ctx.slot(rr, item, Some(counts[i]), bg, Some(UiButton::Quickbar(i)), tip);
                }
            });
        }
    });
}

fn hover_highlight(mut q: Query<(&Interaction, &Base, &mut BackgroundColor), Changed<Interaction>>) {
    for (i, base, mut bg) in &mut q {
        bg.0 = if *i == Interaction::None { base.0 } else { SLOT_HOVER };
    }
}

/// The hand-crafting queue, bottom left; click an entry to cancel it.
fn queue(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    fonts: Res<Fonts>,
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
    let mut ctx =
        Ctx { sprites: &mut sprites, assets: &assets, data: &data, fonts: &fonts, db, research: sim.0.research() };
    commands.entity(*root).despawn_related::<Children>();
    commands.entity(*root).with_children(|p| {
        for (i, job) in c.queue.iter().enumerate() {
            let main = db.recipe(job.recipe).results.first().and_then(|x| match x.what {
                ItemOrFluid::Item(i) => Some(i),
                _ => None,
            });
            ctx.slot(
                p,
                main,
                Some(job.count),
                SLOT,
                Some(UiButton::QueueCancel(i as u32)),
                Some(Tip::Recipe(job.recipe)),
            );
        }
    });
}

// ----- character / entity window -----

fn window(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    fonts: Res<Fonts>,
    names: Res<Names>,
    ui: Res<UiState>,
    local: Res<Local_>,
    chart: Res<crate::chart::Chart>,
    mut images: ResMut<Assets<Image>>,
    mut sprites: ResMut<Sprites>,
    mut root: Single<(Entity, &mut Node), With<WindowRoot>>,
    mut last: Local<String>,
    mut checker: Local<Option<Handle<Image>>>,
) {
    let opened = opened(&sim);
    let show = ui.inventory_open || opened.is_some();
    root.1.display = if show { Display::Flex } else { Display::None };
    let Some(c) = character(&sim) else { return };
    if !show {
        last.clear();
        return;
    }
    let db = sim.0.prototypes();
    let cheat = sim.0.player(LOCAL_PLAYER).is_some_and(|p| p.cheat_mode);
    let entity_sig = opened.and_then(|id| sim.0.entity(id)).map(|e| format!("{:?}", e.state)).unwrap_or_default();
    let belt = opened.and_then(|id| sim.0.belts.get(id)).map(|b| b.item_count());
    let sig = format!(
        "{:?}|{:?}|{}|{}|{}|{:?}|{}|{}|{cheat}|{}",
        c.inventory,
        opened,
        entity_sig,
        local.tab,
        local.choosing_recipe,
        belt,
        chart.range,
        // Progress bars and graphs refresh a few times a second.
        if opened.is_some() { sim.0.tick() / 10 } else { 0 },
        sim.0.research().recipes.iter().filter(|e| **e).count()
    );
    if *last == sig {
        return;
    }
    *last = sig;
    let root = root.0;
    let mut ctx =
        Ctx { sprites: &mut sprites, assets: &assets, data: &data, fonts: &fonts, db, research: sim.0.research() };
    commands.entity(root).despawn_related::<Children>();
    let checker = checker.get_or_insert_with(|| images.add(preview_background())).clone();
    let title = opened
        .and_then(|id| sim.0.entity(id))
        .map(|e| names.entity(e.proto).to_owned())
        .unwrap_or_else(|| "Character".to_owned());
    commands.entity(root).with_children(|w| {
        frame_header(w, &mut ctx, &title);
        w.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(12.0), ..default() }).with_children(
            |w| {
                panel(w, PANEL_W, |p| {
                    ctx.subheading(p, "Character");
                    ctx.inventory(p, &c.inventory, 10, |i| SlotRef::Character(i as u16));
                });
                match opened {
                    Some(id) => panel(w, PANEL_W, |p| {
                        entity_panel(p, &mut ctx, &sim, &names, id, &local, &chart, &mut images, &checker)
                    }),
                    None => panel(w, PANEL_W, |p| crafting_panel(p, &mut ctx, &names, c, local.tab, cheat)),
                }
            },
        );
    });
}

fn crafting_panel(p: &mut ChildSpawnerCommands, ctx: &mut Ctx, names: &Names, c: &Character, tab: usize, cheat: bool) {
    let db = ctx.db;
    let cats = match &db.entity(c.proto).data {
        EntityData::Character { crafting_categories, .. } => crafting_categories.clone(),
        _ => Vec::new(),
    };
    let enabled = &ctx.research.recipes;
    let hand = |r: RecipeId| {
        let rec = db.recipe(r);
        enabled[r.index()]
            && cats.contains(&rec.category)
            && rec.ingredients.iter().all(|i| matches!(i.what, ItemOrFluid::Item(_)))
            && rec.results.iter().all(|x| matches!(x.what, ItemOrFluid::Item(_)))
    };
    let groups: Vec<&MenuGroup> = names.menu.iter().filter(|g| g.rows.iter().flatten().any(|r| hand(*r))).collect();
    let tab = tab.min(groups.len().saturating_sub(1));
    ctx.heading(p, "Crafting");
    // Item-group tabs.
    p.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(2.0), ..default() }).with_children(|t| {
        for (i, g) in groups.iter().enumerate() {
            let d = ctx.data.0.clone();
            let icon = ctx.sprites.get(ctx.assets, ctx.data, &format!("group:{}", g.name), || {
                factorio_data::sprite::icon_of(&d, d.prototype("item-group", &g.name))
            });
            // The game's `filter_group_tab`; the selected one uses its selected set.
            let l = looks();
            let mut tab_entity = t.spawn((
                UiButton::Tab(i),
                Button,
                Tip::Text(names.groups.get(&g.name).cloned().unwrap_or_default()),
                Node {
                    width: Val::Px(71.0),
                    height: Val::Px(64.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
            ));
            if i == tab {
                tab_entity.insert(crate::gui_skin::node_image(&l.tab_selected));
            } else {
                tab_entity.insert((crate::gui_skin::node_image(&l.tab.default), l.tab.clone()));
            }
            tab_entity.with_children(|b| {
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
                        Node { width: Val::Px(64.0), height: Val::Px(64.0), ..default() },
                    ));
                }
            });
        }
    });
    let Some(group) = groups.get(tab) else { return };
    for row in &group.rows {
        let recipes: Vec<RecipeId> = row.iter().copied().filter(|r| hand(*r)).collect();
        if recipes.is_empty() {
            continue;
        }
        grid(p, 10, |g| {
            for r in recipes {
                let can = if cheat { 1 } else { max_craftable(db, &cats, enabled, &c.inventory, r) };
                let main = db.recipe(r).results.first().and_then(|x| match x.what {
                    ItemOrFluid::Item(i) => Some(i),
                    _ => None,
                });
                let bg = if can == 0 { SLOT_RED } else { SLOT };
                ctx.slot(g, main, (can > 0).then_some(can), bg, Some(UiButton::Craft(r)), Some(Tip::Recipe(r)));
            }
        });
    }
}

const STATUS_GREEN: Color = Color::srgb(0.45, 0.85, 0.35);
const STATUS_YELLOW: Color = Color::srgb(0.95, 0.8, 0.3);
const STATUS_RED: Color = Color::srgb(0.95, 0.35, 0.3);

/// Factorio-style status text for a machine.
fn status(sim: &Sim, id: EntityId) -> (&'static str, Color) {
    let db = sim.0.prototypes();
    let Some(e) = sim.0.entity(id) else { return ("", TEXT) };
    let proto = db.entity(e.proto);
    let (green, yellow, red) = (STATUS_GREEN, STATUS_YELLOW, STATUS_RED);
    let no_power = |energy: &factorio_sim::energy::EnergyState| match (energy, proto.energy_source()) {
        (factorio_sim::energy::EnergyState::Burner(b), _) => !b.has_fuel(),
        (_, Some(EnergySource::Electric { .. })) => {
            !sim.0.power.last_consumption.get(&id).is_some_and(|v| v.is_positive())
        }
        _ => false,
    };
    let fuel_word =
        if matches!(proto.energy_source(), Some(EnergySource::Burner { .. })) { "No fuel" } else { "No power" };
    match &e.state {
        EntityState::Drill(d) => {
            if d.output.is_some() {
                ("Output full", yellow)
            } else if factorio_sim::machines::drill_resources(&sim.0, proto, e.position, e.direction).is_empty() {
                ("No minable resources", red)
            } else if no_power(&d.energy) {
                (fuel_word, red)
            } else {
                ("Working", green)
            }
        }
        EntityState::Crafter(c) => {
            if c.recipe.is_none() && !c.furnace {
                ("No recipe", red)
            } else if !c.crafting && c.input.is_empty() {
                ("Waiting for source items", yellow)
            } else if no_power(&c.energy) {
                (fuel_word, red)
            } else if !c.crafting {
                ("Output full", yellow)
            } else {
                ("Working", green)
            }
        }
        EntityState::Inserter(i) => {
            if no_power(&i.energy) && !matches!(i.energy, factorio_sim::energy::EnergyState::Electric { .. }) {
                (fuel_word, red)
            } else if i.hand.is_some() {
                ("Working", green)
            } else {
                ("Waiting for source items", yellow)
            }
        }
        EntityState::Lab(l) => {
            let r = sim.0.research();
            if r.current().is_none() {
                ("No research in progress", yellow)
            } else if l.working {
                ("Working", green)
            } else if no_power(&l.energy) {
                (fuel_word, red)
            } else {
                ("Missing science packs", yellow)
            }
        }
        EntityState::Fluid(f) => {
            if f.last_power.is_positive() {
                ("Working", green)
            } else if f.energy.burner().is_some_and(|b| !b.has_fuel()) {
                ("No fuel", red)
            } else {
                ("Idle", yellow)
            }
        }
        EntityState::Container(_) => ("Normal", green),
        _ => ("", TEXT),
    }
}

fn entity_panel(
    p: &mut ChildSpawnerCommands,
    ctx: &mut Ctx,
    sim: &Sim,
    names: &Names,
    id: EntityId,
    local: &Local_,
    chart: &crate::chart::Chart,
    images: &mut Assets<Image>,
    checker: &Handle<Image>,
) {
    let db = ctx.db;
    let Some(e) = sim.0.entity(id) else { return };
    let proto = db.entity(e.proto);
    // Status: a coloured light and the status text.
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
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.0),
            height: Val::Px(20.0),
            ..default()
        })
        .with_children(|r| {
            ctx.utility(r, light, 16.0);
            ctx.text(r, st, 14.0, Color::WHITE);
        });
    }
    if !matches!(e.state, EntityState::Pole) {
        entity_preview(p, ctx, sim, id, checker);
    }
    let fuel_slots = |p: &mut ChildSpawnerCommands, ctx: &mut Ctx, energy: &factorio_sim::energy::EnergyState| {
        // As in the game: the fuel slots, then a red bar of the fuel left burning.
        if let Some(b) = energy.burner() {
            let db = ctx.db;
            let left = b
                .currently_burning
                .and_then(|i| db.item(i).fuel.as_ref())
                .map(|f| (b.remaining.to_f64_lossy() / f.value.to_f64_lossy().max(1e-9)).clamp(0.0, 1.0))
                .unwrap_or(0.0);
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(12.0),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|r| {
                let n = b.fuel.slots().len().max(1);
                ctx.inventory(r, &b.fuel, n, |i| SlotRef::Opened(EntityInventory::Fuel, i as u16));
                if !b.burnt.slots().is_empty() {
                    let n = b.burnt.slots().len();
                    ctx.inventory(r, &b.burnt, n, |i| SlotRef::Opened(EntityInventory::BurntResult, i as u16));
                }
                r.spawn(Node { flex_grow: 1.0, flex_direction: FlexDirection::Column, ..default() })
                    .with_children(|c| progress_bar(c, left, looks().burning_bar_color));
            });
        }
    };
    match &e.state {
        EntityState::Container(inv) => ctx.inventory(p, inv, 10, |i| SlotRef::Opened(EntityInventory::Main, i as u16)),
        EntityState::Drill(d) => {
            let ticks = factorio_sim::machines::drill_resources(&sim.0, proto, e.position, e.direction)
                .first()
                .and_then(|t| sim.0.surface.resource(*t))
                .and_then(|r| db.entity(r.proto).minable.as_ref())
                .map(|m| m.mining_ticks.to_f64_lossy())
                .unwrap_or(1.0);
            progress_bar(p, d.progress.to_f64_lossy() / ticks, PROGRESS);
            fuel_slots(p, ctx, &d.energy);
        }
        EntityState::Crafter(c) => {
            if !c.furnace {
                // Recipe row: its product and name, and the change-recipe button.
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(8.0),
                    align_items: AlignItems::Center,
                    height: Val::Px(SLOT_PX),
                    ..default()
                })
                .with_children(|r| {
                    let main = c.recipe.and_then(|r| db.recipe(r).results.first()).and_then(|x| match x.what {
                        ItemOrFluid::Item(i) => Some(i),
                        _ => None,
                    });
                    if let Some(i) = main {
                        ctx.icon(r, i, 32.0);
                    }
                    let label = c.recipe.map(|r| names.recipe(r).to_owned()).unwrap_or("No recipe".into());
                    r.spawn((
                        Text::new(label),
                        TextFont { font: ctx.fonts.bold.clone(), font_size: 14.0, ..default() },
                        TextColor(Color::WHITE),
                        Node { flex_grow: 1.0, ..default() },
                    ));
                    let l = looks();
                    r.spawn((
                        UiButton::ChangeRecipe,
                        Button,
                        Tip::Text("Change recipe".into()),
                        Node {
                            width: Val::Px(SLOT_PX),
                            height: Val::Px(SLOT_PX),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        crate::gui_skin::node_image(&l.button.default),
                        l.button.clone(),
                    ))
                    .with_children(|b| ctx.utility(b, "change_recipe", 32.0));
                });
            }
            if !c.furnace && (c.recipe.is_none() || local.choosing_recipe) {
                recipe_chooser(p, ctx, names, proto);
                return;
            }
            if !c.furnace {
                separator(p);
            }
            // Ingredients, progress and products in one row.
            let ticks = c.recipe.map(|r| db.recipe(r).ticks().to_f64_lossy()).unwrap_or(1.0);
            let fraction = (c.progress.to_f64_lossy() / ticks).clamp(0.0, 1.0);
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(12.0),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|r| {
                let n = c.input.slots().len().max(1);
                ctx.inventory(r, &c.input, n, |i| SlotRef::Opened(EntityInventory::Input, i as u16));
                production_bar(r, ctx, fraction);
                let n = c.output.slots().len().max(1);
                ctx.inventory(r, &c.output, n, |i| SlotRef::Opened(EntityInventory::Output, i as u16));
            });
            fuel_slots(p, ctx, &c.energy);
        }
        EntityState::Inserter(i) => {
            if let Some(h) = i.hand {
                ctx.text(p, format!("Holding: {}", names.item(h.item)), 14.0, TEXT);
            }
            fuel_slots(p, ctx, &i.energy);
        }
        EntityState::Fluid(f) => {
            for b in &f.boxes {
                let fluid = b.fluid.map(|x| db.fluid(x).name.clone()).unwrap_or("Empty".into());
                ctx.text(
                    p,
                    format!("{}  {:.0} at {:.0} °C", fluid, b.amount.to_f64_lossy(), b.temperature.to_f64_lossy()),
                    14.0,
                    TEXT,
                );
            }
            if matches!(proto.data, EntityData::Generator { .. }) {
                ctx.text(p, format!("Output: {}", crate::chart::power_text(f.last_power.to_f64_lossy())), 14.0, TEXT);
            }
            fuel_slots(p, ctx, &f.energy);
        }
        EntityState::Pole => network_window(p, ctx, sim, id, chart, images),
        EntityState::Lab(l) => research::lab_panel(p, ctx, names, proto, l),
        _ => {}
    }
    if !matches!(e.state, EntityState::Pole) {
        entity_filler(p);
    }
}

/// The game's `entity_frame_filler`: the striped rows filling the rest of an entity panel.
fn entity_filler(p: &mut ChildSpawnerCommands) {
    let Some(rows) = looks().entity_filler.clone() else { return };
    // Takes only the height left over: the rows are absolutely placed so they never add
    // to the panel's size, and clipped to what is left.
    p.spawn(Node {
        flex_grow: 1.0,
        margin: UiRect { top: Val::Px(-8.0), left: Val::Px(-12.0), right: Val::Px(-12.0), ..default() },
        overflow: Overflow::clip(),
        ..default()
    })
    .with_children(|f| {
        f.spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(8.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(16.0),
            ..default()
        })
        .with_children(|r| {
            for _ in 0..24 {
                r.spawn((
                    Node { height: Val::Px(24.0), flex_shrink: 0.0, ..default() },
                    crate::gui_skin::node_image(&rows),
                ));
            }
        });
    });
}

fn recipe_chooser(
    p: &mut ChildSpawnerCommands,
    ctx: &mut Ctx,
    names: &Names,
    proto: &factorio_sim::proto::EntityProto,
) {
    let db = ctx.db;
    let EntityData::CraftingMachine { crafting_categories, .. } = &proto.data else { return };
    ctx.text(p, "Choose a recipe", 14.0, HEADING);
    for g in &names.menu {
        for row in &g.rows {
            let recipes: Vec<RecipeId> = row
                .iter()
                .copied()
                .filter(|r| {
                    let rec = db.recipe(*r);
                    ctx.research.recipe_enabled(*r)
                        && crafting_categories.contains(&rec.category)
                        && rec.ingredients.iter().all(|i| matches!(i.what, ItemOrFluid::Item(_)))
                        && rec.results.iter().all(|x| matches!(x.what, ItemOrFluid::Item(_)))
                })
                .collect();
            if recipes.is_empty() {
                continue;
            }
            grid(p, 10, |gr| {
                for r in recipes {
                    let main = db.recipe(r).results.first().and_then(|x| match x.what {
                        ItemOrFluid::Item(i) => Some(i),
                        _ => None,
                    });
                    ctx.slot(gr, main, None, SLOT, Some(UiButton::SetRecipe(r)), Some(Tip::Recipe(r)));
                }
            });
        }
    }
}

fn network_window(
    p: &mut ChildSpawnerCommands,
    ctx: &mut Ctx,
    sim: &Sim,
    id: EntityId,
    chart: &crate::chart::Chart,
    images: &mut Assets<Image>,
) {
    use crate::chart::{PALETTE, PRODUCTION, power_text};
    let db = ctx.db;
    let Some(n) = sim.0.power.electric_network_of.get(&id).map(|n| &sim.0.power.electric_networks[*n]) else {
        ctx.text(p, "Not connected", 14.0, TEXT);
        return;
    };
    ctx.text(
        p,
        format!(
            "Satisfaction {:.0}%    Production {} / {}",
            n.satisfaction().to_f64_lossy() * 100.0,
            power_text(n.production.to_f64_lossy()),
            power_text(n.capacity.to_f64_lossy())
        ),
        14.0,
        TEXT,
    );
    progress_bar(p, n.satisfaction().to_f64_lossy(), PROGRESS);
    let Some(stats) = sim.0.power.stats_for(id) else { return };
    crate::chart::draw(images, chart, stats);
    let font = ctx.fonts.regular.clone();
    p.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(4.0), ..default() }).with_children(|row| {
        for (i, name) in ["5s", "1m", "10m"].iter().enumerate() {
            let l = looks();
            let look = if chart.range == i { &l.yellow_slot } else { &l.button };
            row.spawn((
                UiButton::ChartRange(i),
                Button,
                Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)), ..default() },
                crate::gui_skin::node_image(&look.default),
                look.clone(),
            ))
            .with_children(|b| {
                b.spawn((Text::new(*name), TextFont { font: font.clone(), font_size: 13.0, ..default() }));
            });
        }
    });
    p.spawn((
        ImageNode::new(chart.image.clone()),
        Node { width: Val::Px(crate::chart::WIDTH as f32), height: Val::Px(crate::chart::HEIGHT as f32), ..default() },
    ));
    let last = stats.series[chart.range].samples.back().cloned().unwrap_or_default();
    let swatch = |p: &mut ChildSpawnerCommands, c: [u8; 3], text: String| {
        p.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(6.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|r| {
            r.spawn((
                Node { width: Val::Px(10.0), height: Val::Px(10.0), ..default() },
                BackgroundColor(Color::srgb_u8(c[0], c[1], c[2])),
            ));
            ctx.text(r, text, 13.0, TEXT);
        });
    };
    for (pid, prod) in &last.production {
        swatch(p, PRODUCTION, format!("{} (production): {}", db.entity(*pid).name, power_text(prod.to_f64_lossy())));
    }
    for (k, proto) in crate::chart::series_order(stats, chart.range).iter().enumerate() {
        let v = last.consumption.get(proto).map_or(0.0, |v| v.to_f64_lossy());
        swatch(p, PALETTE[k], format!("{}: {}", db.entity(*proto).name, power_text(v)));
    }
}

// ----- tooltip, cursor icon, selection -----

fn tooltip(
    mut commands: Commands,
    hovered: Query<(&Interaction, &Tip)>,
    window: Single<&Window, With<PrimaryWindow>>,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    fonts: Res<Fonts>,
    names: Res<Names>,
    mut sprites: ResMut<Sprites>,
    mut root: Single<(Entity, &mut Node), With<TooltipRoot>>,
    mut last: Local<String>,
) {
    let tip = hovered.iter().find(|(i, _)| **i != Interaction::None).map(|(_, t)| t.clone());
    let Some(tip) = tip else {
        root.1.display = Display::None;
        last.clear();
        return;
    };
    if let Some(c) = window.cursor_position() {
        root.1.display = Display::Flex;
        root.1.left = Val::Px(c.x + 18.0);
        root.1.top = Val::Px(c.y + 18.0);
    }
    let sig = format!("{tip:?}");
    if *last == sig {
        return;
    }
    *last = sig;
    let db = sim.0.prototypes();
    let mut ctx =
        Ctx { sprites: &mut sprites, assets: &assets, data: &data, fonts: &fonts, db, research: sim.0.research() };
    let root = root.0;
    commands.entity(root).despawn_related::<Children>();
    commands.entity(root).with_children(|p| match tip {
        Tip::Text(t) => ctx.text(p, t, 14.0, TEXT),
        Tip::Tech(t) => {
            let r = sim.0.research();
            ctx.heading(p, names.tech(r, t));
            if let Some(unit) = &db.technology(t).unit {
                let count = unit.count_for(r.level[t.index()]);
                let packs: Vec<String> = unit.ingredients.iter().map(|(i, _)| names.item(*i).to_owned()).collect();
                ctx.text(p, format!("{count} × ({})", packs.join(", ")), 14.0, TEXT);
            }
            ctx.text(p, "Click: details   Right click: remove from queue", 12.0, Color::srgb(0.6, 0.6, 0.6));
        }
        Tip::Item(i) => {
            ctx.heading(p, names.item(i).to_owned());
            let item = db.item(i);
            ctx.text(p, format!("Stack size: {}", item.stack_size), 14.0, TEXT);
            if let Some(f) = &item.fuel {
                ctx.text(p, format!("Fuel value: {:.1} MJ", f.value.to_f64_lossy() / 1e6), 14.0, TEXT);
            }
        }
        Tip::Recipe(r) => {
            let rec = db.recipe(r);
            ctx.heading(p, format!("{} (Recipe)", names.recipe(r)));
            ctx.text(p, "Ingredients:", 14.0, HEADING);
            for ing in &rec.ingredients {
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|row| match ing.what {
                    ItemOrFluid::Item(i) => {
                        ctx.icon(row, i, 24.0);
                        ctx.text(row, format!("{} x {}", ing.amount, names.item(i)), 14.0, TEXT);
                    }
                    ItemOrFluid::Fluid(f) => {
                        ctx.text(row, format!("{} x {}", ing.amount, db.fluid(f).name), 14.0, TEXT)
                    }
                });
            }
            ctx.text(p, format!("{} s  Crafting time", rec.energy_required), 14.0, TEXT);
            if rec.results.len() > 1 || rec.results.first().is_some_and(|x| x.amount_max != factorio_sim::Fixed::ONE) {
                ctx.text(p, "Products:", 14.0, HEADING);
                for x in &rec.results {
                    if let ItemOrFluid::Item(i) = x.what {
                        p.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(6.0),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            ctx.icon(row, i, 24.0);
                            ctx.text(row, format!("{} x {}", x.amount_max, names.item(i)), 14.0, TEXT);
                        });
                    }
                }
            }
            ctx.text(
                p,
                "Left click: craft 1   Right click: craft 5   Shift+click: craft all",
                12.0,
                Color::srgb(0.6, 0.6, 0.6),
            );
        }
    });
}

fn cursor_icon(
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    ui: Res<UiState>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut sprites: ResMut<Sprites>,
    mut icon: Single<(&mut ImageNode, &mut Node, &Children), With<CursorIcon>>,
    mut texts: Query<&mut Text>,
) {
    let held = character(&sim).and_then(|c| c.cursor);
    let place = held.is_some_and(|s| sim.0.prototypes().item(s.item).place_result.is_some());
    // Over the world, buildable items show as a ghost instead.
    let show = held.is_some() && (ui.pointer_over_ui || !place);
    let (ref mut image, ref mut node, children) = *icon;
    let (Some(stack), Some(c), true) = (held, window.cursor_position(), show) else {
        node.display = Display::None;
        return;
    };
    node.display = Display::Flex;
    node.left = Val::Px(c.x + 4.0);
    node.top = Val::Px(c.y + 4.0);
    let name = sim.0.prototypes().item(stack.item).name.clone();
    if let Some(l) = sprites.item_icon(&assets, &data, &name) {
        let s = &l.sprite;
        image.image = l.image.clone();
        image.rect = Some(Rect::new(s.x as f32, s.y as f32, (s.x + s.width) as f32, (s.y + s.height) as f32));
    }
    if let Some(child) = children.first()
        && let Ok(mut t) = texts.get_mut(*child)
    {
        t.0 = if stack.count > 1 { stack.count.to_string() } else { String::new() };
    }
}

/// Factorio-style selection corners around the hovered entity.
fn selection_box(sim: Res<Sim>, mouse: Res<MouseWorld>, ui: Res<UiState>, mut gizmos: Gizmos) {
    if ui.pointer_over_ui {
        return;
    }
    let Some(id) = mouse.0.and_then(|p| sim.0.entity_at(p)) else { return };
    let e = sim.0.entity(id).unwrap();
    let proto = sim.0.prototypes().entity(e.proto);
    let area = factorio_sim::world::Simulation::footprint(proto, e.position, e.direction);
    let a = crate::map_to_world(area.left_top);
    let b = crate::map_to_world(area.right_bottom);
    let (x0, x1, y0, y1) = (a.x, b.x, b.y, a.y);
    let len = crate::TILE * 0.3;
    let color = Color::srgb(1.0, 0.85, 0.3);
    for (cx, cy, dx, dy) in [(x0, y0, 1.0, 1.0), (x1, y0, -1.0, 1.0), (x0, y1, 1.0, -1.0), (x1, y1, -1.0, -1.0)] {
        gizmos.line_2d(Vec2::new(cx, cy), Vec2::new(cx + dx * len, cy), color);
        gizmos.line_2d(Vec2::new(cx, cy), Vec2::new(cx, cy + dy * len), color);
    }
}
