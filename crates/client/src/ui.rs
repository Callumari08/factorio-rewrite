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

mod hud;
mod research;
mod tips;
use crate::sprites::Sprites;
use crate::{Data, LOCAL_PLAYER, PendingInputs, Sim};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(Startup, (load_names, setup, research::setup, hud::setup).chain())
            .add_systems(PostUpdate, font_weights.before(bevy::ui::UiSystems::Prepare))
            .add_systems(
                Update,
                (
                    mining_bar,
                    override_slider,
                    drag_spread.after(clicks),
                    research::tree_view,
                    research::search_typing.before(clicks),
                ),
            )
            .add_systems(
                Update,
                (
                    pointer_over_ui,
                    clicks,
                    hud,
                    quickbar,
                    queue,
                    window,
                    live_bars,
                    hide_hud,
                    hud::side_menu,
                    hud::minimap,
                    hud::character_panel,
                    hud::shortcut_bar,
                    research::scroll,
                    research::window,
                    research::hud,
                    hover_highlight,
                    tooltip,
                    entity_info,
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
/// The quickbar's width: row numbers, ten slots, padding.
/// Measured from the game at 125 % (585 × 120 px): 8 px padding, the row buttons, a 7 px
/// gap, ten slots with 4 px between the fifth and sixth.
const QUICKBAR_W: f32 = 8.0 + SLOT_PX + 7.0 + SLOT_PX * 10.0 + 4.0 + 8.0;
const QUICKBAR_H: f32 = SLOT_PX * 2.0 + 16.0;
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
pub(super) struct EntityInfoRoot;
#[derive(Component)]
struct MiningBar;
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
    TechSearch,
    ContainerLimit,
    InserterUseFilters,
    InserterBlacklist,
    InserterFilter(u8),
    InserterOverride,
    /// The override slider (the hand size it spans to).
    InserterOverrideSlider(u32),
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
    /// The next slot click sets the opened container's limit.
    limit_mode: bool,
    /// Slots swept by the current drag with the cursor stack.
    drag: Vec<SlotRef>,
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

fn setup(mut commands: Commands, fonts: Res<Fonts>, mut images: ResMut<Assets<Image>>) {
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
            // Bottom centre, touching the screen edge as in the game.
            bottom: Val::Px(0.0),
            left: Val::Percent(50.0),
            margin: UiRect::left(Val::Px(-QUICKBAR_W / 2.0)),
            width: Val::Px(QUICKBAR_W),
            height: Val::Px(QUICKBAR_H),
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(7.0),
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        },
        crate::gui_skin::node_image(&looks().frame),
    ));
    // The mining bar: a 13 px strip resting on the quickbar's top edge, the quickbar's
    // width, black behind the orange fill (measured from the game).
    let mining_fill = images.add(mining_gradient());
    commands
        .spawn((
            MiningBar,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(QUICKBAR_H),
                left: Val::Percent(50.0),
                margin: UiRect::left(Val::Px(-QUICKBAR_W / 2.0)),
                width: Val::Px(QUICKBAR_W),
                height: Val::Px(13.0),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::BLACK),
            Pickable::IGNORE,
        ))
        .with_children(|b| {
            b.spawn((
                LiveFill(Live::Mining),
                Node { width: Val::Percent(0.0), height: Val::Px(13.0), ..default() },
                ImageNode::new(mining_fill),
            ));
        });
    // Windows are centred on the screen; the root only lays them out (each frame catches
    // the mouse itself).
    commands.spawn((
        WindowRoot,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            display: Display::None,
            ..default()
        },
        Pickable::IGNORE,
    ));
    commands.spawn((
        QueueRoot,
        Interaction::default(),
        Node {
            position_type: PositionType::Absolute,
            // Along the bottom, right of the character panel.
            bottom: Val::Px(4.0),
            left: Val::Px(hud::CHARACTER_PANEL_W),
            flex_direction: FlexDirection::Row,
            ..default()
        },
    ));
    let (mut tip_node, tip_image) = tips::tip_frame();
    tip_node.position_type = PositionType::Absolute;
    tip_node.display = Display::None;
    commands.spawn((TooltipRoot, tip_node, tip_image, GlobalZIndex(10), Pickable::IGNORE));

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
    // Middle click on a character slot sets its filter to the item in it (or held), or
    // clears it, as in the game.
    if mouse.just_pressed(MouseButton::Middle)
        && let Some((_, UiButton::Slot(SlotRef::Character(i)))) = q.iter().find(|(i, _)| **i != Interaction::None)
        && let Some(c) = character(&sim)
    {
        let i = *i;
        let item = if c.inventory.filter(i as usize).is_some() {
            None
        } else {
            c.inventory.slot(i as usize).map(|s| s.item).or(c.cursor.map(|s| s.item))
        };
        pending.push(InputAction::SetSlotFilter { slot: i, item });
        return;
    }
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
        (UiButton::Slot(SlotRef::Opened(EntityInventory::Main, i)), SimButton::Left) if local.limit_mode => {
            local.limit_mode = false;
            pending.push(InputAction::SetContainerLimit(Some(i)));
        }
        (UiButton::ContainerLimit, SimButton::Left) => local.limit_mode = !local.limit_mode,
        (UiButton::InserterUseFilters | UiButton::InserterBlacklist, _) => {
            if let Some(EntityState::Inserter(i)) = opened(&sim).and_then(|id| sim.0.entity(id)).map(|e| &e.state) {
                let (use_filters, blacklist) = match target {
                    UiButton::InserterUseFilters => (!i.use_filters, i.blacklist),
                    _ => (i.use_filters, !i.blacklist),
                };
                pending.push(InputAction::SetInserterFilterMode { use_filters, blacklist });
            }
        }
        (UiButton::InserterFilter(k), SimButton::Left) => {
            pending.push(InputAction::SetInserterFilter { index: k, item: held });
        }
        (UiButton::InserterFilter(k), SimButton::Right) => {
            pending.push(InputAction::SetInserterFilter { index: k, item: None });
        }
        (UiButton::InserterOverride, _) => {
            if let Some(e) = opened(&sim).and_then(|id| sim.0.entity(id))
                && let EntityState::Inserter(i) = &e.state
            {
                let size = factorio_sim::machines::hand_size(&sim.0, sim.0.prototypes().entity(e.proto));
                pending.push(InputAction::SetInserterStackOverride(if i.stack_override.is_some() {
                    None
                } else {
                    Some(size)
                }));
            }
        }
        (UiButton::InserterOverrideSlider(_), _) => {}
        (UiButton::ContainerLimit, SimButton::Right) => {
            local.limit_mode = false;
            pending.push(InputAction::SetContainerLimit(None));
        }
        // Holding a stack, a left press on a free (or same-item) slot starts a drag-spread
        // (see `drag_spread`); everything else is an ordinary click.
        // The character's own inventory sorts itself, so dragging only spreads into the
        // opened entity's slots.
        (UiButton::Slot(slot @ SlotRef::Opened(..)), SimButton::Left)
            if !shift && !ctrl && held.is_some() && slot_takes(&sim, slot, held.unwrap()) =>
        {
            local.drag = vec![slot];
            pending.push(InputAction::SpreadCursor { slots: vec![slot] });
        }
        (UiButton::Slot(slot @ SlotRef::Opened(..)), SimButton::Right) if !shift && !ctrl && held.is_some() => {
            local.drag = vec![slot];
            pending.push(InputAction::ClickSlot { slot, button: SimButton::Right, shift, ctrl });
        }
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
        (UiButton::SelectTech(t), SimButton::Left) => {
            tech.selected = Some(t);
            // The tree recentres on the new selection.
            tech.pan = Vec2::ZERO;
        }
        // Right click on a queued technology removes it, as in the game's queue.
        (UiButton::SelectTech(t), SimButton::Right) => pending.push(InputAction::DequeueResearch(t)),
        (UiButton::QueueTech(t), _) => pending.push(InputAction::QueueResearch { tech: t, front: shift }),
        (UiButton::DequeueTech(t), _) => pending.push(InputAction::DequeueResearch(t)),
        (UiButton::CloseWindow, _) => {
            ui.inventory_open = false;
            pending.push(InputAction::OpenEntity(None));
        }
        (UiButton::TechSearch, _) => {
            tech.search = if tech.search.is_some() { None } else { Some(String::new()) };
            ui.typing = tech.search.is_some();
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
    // Mining progress is the bar on the quickbar, crafting the highlighted queue slot.
    let status = ui.status.clone();
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
    /// The button hovered before a rebuild (its `Debug` text): its replacement starts
    /// hovered, so a rebuild never blinks the highlight or the tooltip.
    hovered: Option<String>,
}

impl Ctx<'_> {
    fn text(&self, p: &mut ChildSpawnerCommands, s: impl Into<String>, size: f32, color: Color) {
        p.spawn((
            Text::new(s.into()),
            TextFont { font: self.fonts.regular.clone(), font_size: size, ..default() },
            TextColor(color),
        ));
    }

    /// A panel's caption (the "Character" over the inventory).
    fn subheading(&self, p: &mut ChildSpawnerCommands, s: impl Into<String>) {
        p.spawn((
            Text::new(s.into()),
            TextFont { font: self.fonts.regular.clone(), font_size: 15.0, ..default() },
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
        self.icon_tinted(p, item, size, Color::WHITE);
    }

    fn icon_tinted(&mut self, p: &mut ChildSpawnerCommands, item: ItemId, size: f32, color: Color) {
        let name = self.db.item(item).name.clone();
        if let Some(icon) = self.sprites.item_icon(self.assets, self.data, &name) {
            let s = &icon.sprite;
            p.spawn((
                ImageNode {
                    image: icon.image.clone(),
                    rect: Some(Rect::new(s.x as f32, s.y as f32, (s.x + s.width) as f32, (s.y + s.height) as f32)),
                    color,
                    ..default()
                },
                Node { width: Val::Px(size), height: Val::Px(size), ..default() },
                Pickable::IGNORE,
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
        self.slot_ext(p, item, count, bg, button, tip, false);
    }

    /// [`Ctx::slot`], optionally showing the hand (where the cursor's stack came from).
    #[allow(clippy::too_many_arguments)]
    fn slot_ext(
        &mut self,
        p: &mut ChildSpawnerCommands,
        item: Option<ItemId>,
        count: Option<u32>,
        bg: Color,
        button: Option<UiButton>,
        tip: Option<Tip>,
        hand: bool,
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
            if self.hovered.as_deref() == Some(format!("{b:?}").as_str()) {
                e.insert((Interaction::Hovered, crate::gui_skin::node_image(&look.hovered)));
            }
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
            } else if hand {
                self.utility(c, "hand", 32.0);
            }
            if let Some(n) = count.filter(|n| *n >= 1) {
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
            for i in 0..inv.len() {
                self.inventory_slot(g, inv, i, &make);
            }
        });
    }

    /// One slot of an inventory grid: its contents, the hand, a filter's pale item, and
    /// the red of slots past a container's limit.
    fn inventory_slot(
        &mut self,
        g: &mut ChildSpawnerCommands,
        inv: &Inventory,
        i: usize,
        make: &dyn Fn(usize) -> SlotRef,
    ) {
        let s = &inv.slots()[i];
        let hand = s.is_none() && inv.reserved() == Some(i);
        // Slots past a container's limit are red; the first shows the limit mark.
        let barred = inv.bar().is_some_and(|b| i >= b);
        let bg = if barred { SLOT_RED } else { INV };
        g.spawn(Node { width: Val::Px(SLOT_PX), height: Val::Px(SLOT_PX), ..default() }).with_children(|c| {
            self.slot_ext(c, s.map(|s| s.item), s.map(|s| s.count), bg, Some(UiButton::Slot(make(i))), None, hand);
            // An empty filtered slot shows its item pale.
            if let (None, Some(f)) = (s, inv.filter(i)) {
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
                .with_children(|m| self.icon_tinted(m, f, 32.0, Color::srgba(1.0, 1.0, 1.0, 0.35)));
            }
            if inv.bar() == Some(i) && s.is_none() {
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
                .with_children(|m| self.utility(m, "set_bar_slot", 32.0));
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
/// A window frame (the game's `frame` style) with a title bar, holding `f`'s contents.
fn frame(
    w: &mut ChildSpawnerCommands,
    ctx: &mut Ctx,
    title: &str,
    buttons: &[&str],
    f: impl FnOnce(&mut ChildSpawnerCommands, &mut Ctx),
) {
    w.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect { left: Val::Px(8.0), right: Val::Px(8.0), top: Val::Px(4.0), bottom: Val::Px(8.0) },
            ..default()
        },
        Interaction::default(),
        crate::gui_skin::node_image(&looks().frame),
    ))
    .with_children(|w| {
        frame_header(w, ctx, title, buttons);
        f(w, ctx);
    });
}

/// A frame's title bar: the title, the striped draggable filler and the action buttons
/// (`search`, `close`).
fn frame_header(w: &mut ChildSpawnerCommands, ctx: &mut Ctx, title: &str, buttons: &[&str]) {
    let l = looks();
    w.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        // Measured from the game at 125 %: 10 px (at 100 %) from the filler's bottom edge to
        // the panels.
        height: Val::Px(37.0),
        padding: UiRect::bottom(Val::Px(9.0)),
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
        for b in buttons {
            let mut e = h.spawn((
                Node {
                    width: Val::Px(24.0),
                    height: Val::Px(24.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                crate::gui_skin::node_image(&l.frame_button.default),
                l.frame_button.clone(),
                Button,
            ));
            match *b {
                "close" => e.insert((UiButton::CloseWindow, Tip::Text("Close".into()))),
                "circuit_network_panel" => e.insert(Tip::Text("Circuit network connection".into())),
                "logistic_network_panel_white" => e.insert(Tip::Text("Logistic network connection".into())),
                _ => e.insert(Tip::Text("Search".into())),
            };
            e.with_children(|c| ctx.utility(c, b, 16.0));
        }
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
            draw_layers(v, look, Vec2::new(PREVIEW_W, PREVIEW_H) / 2.0);
        });
}

/// The entity on a plain dark ground (the top of the info panel).
fn entity_picture(p: &mut ChildSpawnerCommands, ctx: &mut Ctx, sim: &Sim, id: EntityId, size: Vec2) {
    let look = crate::render::entity_look(sim, ctx.data, ctx.sprites, ctx.assets, id);
    p.spawn((
        Node { width: Val::Px(size.x), height: Val::Px(size.y), overflow: Overflow::clip(), ..default() },
        BackgroundColor(Color::srgb(0.12, 0.12, 0.12)),
    ))
    .with_children(|v| draw_layers(v, look, size / 2.0));
}

/// An entity's sprite layers, centred on `centre`.
fn draw_layers(v: &mut ChildSpawnerCommands, look: Option<crate::render::Look>, centre: Vec2) {
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
}

/// The game's `production_progressbar`: a 24 px bar with the percentage inside it.
fn production_bar(p: &mut ChildSpawnerCommands, ctx: &Ctx, fraction: f64, live: Live) {
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
            LiveFill(live),
        ));
        b.spawn((
            Text::new(format!("{:.0}%", fraction * 100.0)),
            TextFont { font: ctx.fonts.regular.clone(), font_size: 14.0, ..default() },
            TextColor(if fraction > 0.9 { Color::BLACK } else { Color::WHITE }),
            LiveText(live),
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
/// [`progress_bar`] whose fill follows a [`Live`] value.
fn progress_bar_live(p: &mut ChildSpawnerCommands, fraction: f64, color: Color, live: Option<Live>) {
    let l = looks();
    p.spawn((
        Node { width: Val::Percent(100.0), height: Val::Px(8.0), ..default() },
        crate::gui_skin::node_image(&l.bar_background),
    ))
    .with_children(|b| {
        let mut bar = crate::gui_skin::node_image(&l.bar);
        bar.color = color;
        let mut fill = b.spawn((
            Node {
                width: Val::Percent((fraction.clamp(0.0, 1.0) * 100.0) as f32),
                height: Val::Percent(100.0),
                ..default()
            },
            bar,
        ));
        if let Some(live) = live {
            fill.insert(LiveFill(live));
        }
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
    buttons: Query<(&Interaction, &UiButton)>,
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
    let mut ctx = Ctx {
        sprites: &mut sprites,
        assets: &assets,
        data: &data,
        fonts: &fonts,
        db,
        research: sim.0.research(),
        hovered: hovered_button(&buttons),
    };
    commands.entity(*root).despawn_related::<Children>();
    commands.entity(*root).with_children(|r| {
        // Row numbers.
        r.spawn(Node { flex_direction: FlexDirection::Column, ..default() }).with_children(|col| {
            for row in 0..QUICKBAR_SLOTS / 10 {
                let l = looks();
                col.spawn((
                    Node {
                        width: Val::Px(SLOT_PX),
                        height: Val::Px(SLOT_PX),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    crate::gui_skin::node_image(&l.button.default),
                    l.button.clone(),
                    Button,
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new((row + 1).to_string()),
                        TextFont { font: ctx.fonts.bold.clone(), font_size: 14.0, ..default() },
                        TextColor(Color::BLACK),
                    ));
                });
            }
        });
        r.spawn(Node { flex_direction: FlexDirection::Column, ..default() }).with_children(|r| {
            for row in 0..QUICKBAR_SLOTS / 10 {
                r.spawn(Node { flex_direction: FlexDirection::Row, ..default() }).with_children(|rr| {
                    for k in 0..10 {
                        let i = row * 10 + k;
                        let item = p.quickbar[i];
                        let tip = item.map(Tip::Item).or(Some(Tip::Text(format!(
                            "Quickbar slot {}: click while holding an item to assign; right click clears",
                            (k + 1) % 10
                        ))));
                        let bg = if item.is_some() && counts[i] == 0 { SLOT_RED } else { INV };
                        if k == 5 {
                            // The game splits each row into two halves.
                            rr.spawn(Node { width: Val::Px(4.0), ..default() });
                        }
                        ctx.slot(rr, item, Some(counts[i]), bg, Some(UiButton::Quickbar(i)), tip);
                    }
                });
            }
        });
    });
}

/// Gives text in the bold and semibold fonts their weight. Bevy finds a font face by
/// family and weight, and all three files are "Titillium Web": without the weight it
/// would draw them all in the regular face.
fn font_weights(fonts: Res<Fonts>, mut q: Query<&mut TextFont, Changed<TextFont>>) {
    for mut f in &mut q {
        let w = if f.font == fonts.bold {
            bevy::text::FontWeight::BOLD
        } else if f.font == fonts.semibold {
            bevy::text::FontWeight::SEMIBOLD
        } else {
            continue;
        };
        if f.weight != w {
            f.weight = w;
        }
    }
}

/// Whether a slot is free for, or already holds, `item` (where a left drag can put it).
fn slot_takes(sim: &Sim, slot: SlotRef, item: ItemId) -> bool {
    factorio_sim::cursor::read_slot(&sim.0, LOCAL_PLAYER, slot).is_some_and(|c| c.is_none_or(|s| s.item == item))
}

/// Dragging the inserter's stack size slider sets the override.
fn override_slider(
    mouse: Res<ButtonInput<MouseButton>>,
    q: Query<(&Interaction, &UiButton, &bevy::ui::RelativeCursorPosition)>,
    mut pending: ResMut<PendingInputs>,
    mut last: Local<Option<u32>>,
) {
    if !mouse.pressed(MouseButton::Left) {
        *last = None;
        return;
    }
    for (i, b, cursor) in &q {
        if let (Interaction::Pressed, UiButton::InserterOverrideSlider(max), Some(p)) = (i, b, cursor.normalized) {
            let v = 1 + (((p.x + 0.5).clamp(0.0, 1.0) * (*max - 1) as f32).round() as u32);
            if *last != Some(v) {
                *last = Some(v);
                pending.push(InputAction::SetInserterStackOverride(Some(v)));
            }
        }
    }
}

/// Dragging with the cursor stack, as in the game: with the left button held, the stack
/// is spread evenly over every slot swept; with the right, one item goes into each.
fn drag_spread(
    mouse: Res<ButtonInput<MouseButton>>,
    q: Query<(&Interaction, &UiButton)>,
    mut local: ResMut<Local_>,
    mut pending: ResMut<PendingInputs>,
) {
    if local.drag.is_empty() {
        return;
    }
    let left = mouse.pressed(MouseButton::Left);
    let right = mouse.pressed(MouseButton::Right);
    if !left && !right {
        local.drag.clear();
        pending.push(InputAction::EndSpread);
        return;
    }
    let hovered = q.iter().find_map(|(i, b)| match (i, b) {
        (Interaction::None, _) => None,
        (_, UiButton::Slot(s @ SlotRef::Opened(..))) => Some(*s),
        _ => None,
    });
    let Some(slot) = hovered else { return };
    if local.drag.contains(&slot) {
        return;
    }
    local.drag.push(slot);
    if left {
        pending.push(InputAction::SpreadCursor { slots: local.drag.clone() });
    } else {
        pending.push(InputAction::ClickSlot { slot, button: SimButton::Right, shift: false, ctrl: false });
    }
}

/// The hovered button's `Debug` text (see [`Ctx::hovered`]).
fn hovered_button(q: &Query<(&Interaction, &UiButton)>) -> Option<String> {
    q.iter().find(|(i, _)| **i != Interaction::None).map(|(_, b)| format!("{b:?}"))
}

/// The technology screen covers the HUD, as in the game.
fn hide_hud(ui: Res<UiState>, mut q: Query<&mut Node, Or<(With<QuickbarRoot>, With<QueueRoot>, With<hud::HudRoot>)>>) {
    for mut n in &mut q {
        let d = if ui.tech_open { Display::None } else { Display::Flex };
        if n.display != d {
            n.display = d;
        }
    }
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
    let mut ctx = Ctx {
        sprites: &mut sprites,
        assets: &assets,
        data: &data,
        fonts: &fonts,
        db,
        research: sim.0.research(),
        hovered: None,
    };
    commands.entity(*root).despawn_related::<Children>();
    commands.entity(*root).with_children(|p| {
        for (i, job) in c.queue.iter().enumerate() {
            let main = db.recipe(job.recipe).results.first().and_then(|x| match x.what {
                ItemOrFluid::Item(i) => Some(i),
                _ => None,
            });
            // The one being crafted is highlighted, as in the game.
            ctx.slot(
                p,
                main,
                Some(job.count),
                if i == 0 { TAB_SELECTED } else { SLOT },
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
    buttons: Query<(&Interaction, &UiButton)>,
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
    // Rebuilt only when what the window shows changes structurally; progress bars update
    // in place (`live_bars`), so hovered slots keep their highlight and tooltip.
    let entity_sig = opened.map(|id| structure_sig(&sim, id)).unwrap_or_default();
    let belt = opened.and_then(|id| sim.0.belts.get(id)).map(|b| b.item_count());
    let sig = format!(
        "{:?}|{:?}|{}|{}|{}|{}|{:?}|{}|{cheat}|{}",
        c.inventory,
        opened,
        entity_sig,
        local.tab,
        local.choosing_recipe,
        local.limit_mode,
        belt,
        chart.range,
        sim.0.research().recipes.iter().filter(|e| **e).count()
    );
    if *last == sig {
        return;
    }
    *last = sig;
    let root = root.0;
    let mut ctx = Ctx {
        sprites: &mut sprites,
        assets: &assets,
        data: &data,
        fonts: &fonts,
        db,
        research: sim.0.research(),
        hovered: hovered_button(&buttons),
    };
    commands.entity(root).despawn_related::<Children>();
    let checker = checker.get_or_insert_with(|| images.add(preview_background())).clone();
    let title = opened
        .and_then(|id| sim.0.entity(id))
        .map(|e| names.entity(e.proto).to_owned())
        .unwrap_or_else(|| "Character".to_owned());
    commands.entity(root).with_children(|w| match opened {
        // An entity: one frame, the character's inventory beside the entity's panel.
        // Entities without item slots: just their panel, no inventory (as in the game).
        Some(id) if !shows_inventory(&sim, id) => {
            let pole = sim.0.entity(id).is_some_and(|e| matches!(e.state, EntityState::Pole));
            let title = if pole { "Electric network info".to_owned() } else { title.clone() };
            frame(w, &mut ctx, &title, &network_buttons(&sim, &data, id), |w, ctx| {
                panel(w, if pole { NETWORK_COL_W * 3.0 + 24.0 + 24.0 } else { PANEL_W }, |p| {
                    entity_panel(p, ctx, &sim, &names, id, &local, &chart, &mut images, &checker)
                });
            })
        }
        Some(id) => frame(w, &mut ctx, &title, &network_buttons(&sim, &data, id), |w, ctx| {
            w.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(12.0), ..default() }).with_children(
                |w| {
                    panel(w, PANEL_W, |p| {
                        ctx.subheading(p, "Character");
                        ctx.inventory(p, &c.inventory, 10, |i| SlotRef::Character(i as u16));
                    });
                    panel(w, PANEL_W, |p| {
                        entity_panel(p, ctx, &sim, &names, id, &local, &chart, &mut images, &checker)
                    });
                },
            );
        }),
        // The character window: the Character and Crafting frames side by side.
        // Both frames share their top and height, as in the game.
        None => {
            w.spawn(Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Stretch, ..default() })
                .with_children(|w| {
                    frame(w, &mut ctx, "Character", &[], |w, ctx| {
                        panel(w, PANEL_W, |p| {
                            // Toolbar: the colour picker and the player's colour.
                            p.spawn(Node {
                                flex_direction: FlexDirection::Row,
                                justify_content: JustifyContent::FlexEnd,
                                column_gap: Val::Px(4.0),
                                height: Val::Px(28.0),
                                ..default()
                            })
                            .with_children(|t| {
                                let l = looks();
                                t.spawn((
                                    Node {
                                        width: Val::Px(28.0),
                                        height: Val::Px(28.0),
                                        justify_content: JustifyContent::Center,
                                        align_items: AlignItems::Center,
                                        ..default()
                                    },
                                    crate::gui_skin::node_image(&l.button.default),
                                    l.button.clone(),
                                    Button,
                                    Tip::Text("Character colour".into()),
                                ))
                                .with_children(|b| ctx.utility(b, "color_picker", 24.0));
                                t.spawn((
                                    Node { width: Val::Px(28.0), height: Val::Px(28.0), ..default() },
                                    BackgroundColor(Color::srgb(0.869, 0.5, 0.130)),
                                ));
                            });
                            ctx.inventory(p, &c.inventory, 10, |i| SlotRef::Character(i as u16));
                        });
                    });
                    frame(w, &mut ctx, "Crafting", &["search", "close"], |w, ctx| {
                        crafting_panel(w, ctx, &names, c, local.tab, cheat);
                    });
                });
        }
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
    // As in the game, every enabled recipe is listed; those that cannot be crafted now
    // (another crafting category, fluids, or missing ingredients) are red, without a count.
    let shown = |r: RecipeId| enabled[r.index()];
    let groups: Vec<&MenuGroup> = names.menu.iter().filter(|g| g.rows.iter().flatten().any(|r| shown(*r))).collect();
    let tab = tab.min(groups.len().saturating_sub(1));
    // Item-group tabs (the game's `filter_group_tab`), stretched to fill the row.
    p.spawn(Node { flex_direction: FlexDirection::Row, width: Val::Px(PANEL_W), ..default() }).with_children(|t| {
        for (i, g) in groups.iter().enumerate() {
            let d = ctx.data.0.clone();
            let icon = ctx.sprites.get(ctx.assets, ctx.data, &format!("group:{}", g.name), || {
                factorio_data::sprite::icon_of(&d, d.prototype("item-group", &g.name))
            });
            let l = looks();
            let mut tab_entity = t.spawn((
                UiButton::Tab(i),
                Button,
                Tip::Text(names.groups.get(&g.name).cloned().unwrap_or_default()),
                Node {
                    flex_grow: 1.0,
                    min_width: Val::Px(71.0),
                    height: Val::Px(72.0),
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
                        Pickable::IGNORE,
                    ));
                }
            });
        }
    });
    // The recipes: one row per subgroup in a deep pane tiled with empty cells.
    panel(p, PANEL_W, |p| {
        p.spawn(Node { flex_direction: FlexDirection::Column, flex_grow: 1.0, ..default() }).with_children(|pane| {
            crate::gui_skin::backdrop(pane, &looks().deep_in_shallow);
            let mut rows = 0;
            if let Some(group) = groups.get(tab) {
                for row in &group.rows {
                    let recipes: Vec<RecipeId> = row.iter().copied().filter(|r| shown(*r)).collect();
                    for chunk in recipes.chunks(10) {
                        rows += 1;
                        grid(pane, 10, |g| {
                            for r in chunk {
                                let r = *r;
                                let by_hand = hand(r);
                                let can = if !by_hand {
                                    0
                                } else if cheat {
                                    1
                                } else {
                                    max_craftable(db, &cats, enabled, &c.inventory, r)
                                };
                                let main = db.recipe(r).results.first().and_then(|x| match x.what {
                                    ItemOrFluid::Item(i) => Some(i),
                                    _ => None,
                                });
                                // Red whenever it cannot be crafted now: another crafting
                                // category, or not enough ingredients.
                                let bg = if can > 0 { SLOT } else { SLOT_RED };
                                ctx.slot(
                                    g,
                                    main,
                                    (can > 0).then_some(can),
                                    bg,
                                    Some(UiButton::Craft(r)),
                                    Some(Tip::Recipe(r)),
                                );
                            }
                            for _ in chunk.len()..10 {
                                empty_cell(g);
                            }
                        });
                    }
                }
            }
            // At least seven rows, as the game's pane.
            for _ in rows..7 {
                grid(pane, 10, |g| {
                    for _ in 0..10 {
                        empty_cell(g);
                    }
                });
            }
        });
    });
}

/// A slot showing a pale placeholder picture (empty module, inserter hand, science pack).
fn ghost_slot(p: &mut ChildSpawnerCommands, ctx: &mut Ctx, sprite: &str) {
    let look = &looks().slot;
    p.spawn((
        Node {
            width: Val::Px(SLOT_PX),
            height: Val::Px(SLOT_PX),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        crate::gui_skin::node_image(&look.default),
        look.clone(),
        Interaction::default(),
    ))
    .with_children(|c| ctx.utility(c, sprite, 32.0));
}

/// A row of ten slot cells: `f` spawns the first `n`, empty cells fill the rest.
fn slot_row(
    p: &mut ChildSpawnerCommands,
    ctx: &mut Ctx,
    n: usize,
    f: impl FnOnce(&mut ChildSpawnerCommands, &mut Ctx),
) {
    p.spawn(Node { flex_direction: FlexDirection::Row, ..default() }).with_children(|r| {
        f(r, ctx);
        for _ in n..10 {
            empty_cell(r);
        }
    });
}

/// The entity's module slots (empty: modules are not implemented yet), if it has any.
fn module_row(p: &mut ChildSpawnerCommands, ctx: &mut Ctx, proto: &factorio_sim::proto::EntityProto) {
    let n = ctx.data.0.prototype(&proto.kind, &proto.name).get("module_slots").as_i64().unwrap_or(0) as usize;
    if n == 0 {
        return;
    }
    slot_row(p, ctx, n, |r, ctx| {
        for _ in 0..n {
            ghost_slot(r, ctx, "empty_module_slot");
        }
    });
}

/// The game's checkbox: the box, with the check mark when `on`.
fn checkbox(p: &mut ChildSpawnerCommands, on: bool, button: UiButton) {
    let l = looks();
    p.spawn((
        Node { width: Val::Px(14.0), height: Val::Px(14.0), ..default() },
        crate::gui_skin::node_image(if on { &l.checkbox_checked } else { &l.checkbox.default }),
        button,
        Button,
    ))
    .with_children(|c| {
        if on {
            c.spawn((
                Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
                crate::gui_skin::node_image(&l.checkmark),
                Pickable::IGNORE,
            ));
        }
    });
}

/// The inserter's filter section, as in the game: "Use filters" with the whitelist /
/// blacklist switch on the left, the filter slots in a deep frame on the right.
fn inserter_filters(p: &mut ChildSpawnerCommands, ctx: &mut Ctx, ins: &factorio_sim::machines::InserterState) {
    p.spawn(Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, ..default() }).with_children(
        |row| {
            row.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                flex_grow: 1.0,
                ..default()
            })
            .with_children(|col| {
                col.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|r| {
                    checkbox(r, ins.use_filters, UiButton::InserterUseFilters);
                    ctx.text(r, "Use filters", 14.0, Color::WHITE);
                });
                // Whitelist [switch] Blacklist.
                // The chosen side is bold and white, the other grey.
                let side = |r: &mut ChildSpawnerCommands, ctx: &Ctx, s: &str, on: bool| {
                    r.spawn((
                        Text::new(s),
                        TextFont {
                            font: if on { ctx.fonts.bold.clone() } else { ctx.fonts.regular.clone() },
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(if on { Color::WHITE } else { Color::srgb(0.55, 0.55, 0.55) }),
                    ));
                };
                col.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|r| {
                    side(r, ctx, "Whitelist", !ins.blacklist);
                    r.spawn((
                        Node {
                            width: Val::Px(30.0),
                            height: Val::Px(14.0),
                            justify_content: if ins.blacklist {
                                JustifyContent::FlexEnd
                            } else {
                                JustifyContent::FlexStart
                            },
                            padding: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(7.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.1, 0.1, 0.1)),
                        UiButton::InserterBlacklist,
                        Button,
                    ))
                    .with_children(|sw| {
                        sw.spawn((
                            Node {
                                width: Val::Px(12.0),
                                height: Val::Px(12.0),
                                border_radius: BorderRadius::all(Val::Px(6.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.6, 0.6, 0.6)),
                            Pickable::IGNORE,
                        ));
                    });
                    side(r, ctx, "Blacklist", ins.blacklist);
                });
            });
            row.spawn(Node { flex_direction: FlexDirection::Row, padding: UiRect::all(Val::Px(4.0)), ..default() })
                .with_children(|f| {
                    crate::gui_skin::backdrop(f, &looks().deep_in_shallow);
                    for (k, filter) in ins.filters.iter().enumerate() {
                        ctx.slot(
                            f,
                            *filter,
                            None,
                            SLOT,
                            Some(UiButton::InserterFilter(k as u8)),
                            Some(Tip::Text("Click with an item to set the filter; right click clears it".into())),
                        );
                    }
                });
        },
    );
}

/// The "Override stack size" row: checkbox, slider and value.
fn inserter_stack_override(
    p: &mut ChildSpawnerCommands,
    ctx: &mut Ctx,
    sim: &Sim,
    proto: &factorio_sim::proto::EntityProto,
    ins: &factorio_sim::machines::InserterState,
) {
    let max = factorio_sim::machines::hand_size(&sim.0, proto);
    let value = factorio_sim::machines::effective_hand_size(&sim.0, proto, ins);
    let l = looks();
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        ..default()
    })
    .with_children(|r| {
        checkbox(r, ins.stack_override.is_some(), UiButton::InserterOverride);
        r.spawn((
            Text::new("Override stack size"),
            TextFont { font: ctx.fonts.regular.clone(), font_size: 14.0, ..default() },
            TextColor(Color::WHITE),
            TextLayout::new_with_no_wrap(),
            Node { flex_grow: 1.0, ..default() },
        ));
        let fraction = if max > 1 { (value - 1) as f32 / (max - 1) as f32 } else { 0.0 };
        r.spawn((
            Node { width: Val::Px(160.0), height: Val::Px(12.0), align_items: AlignItems::Center, ..default() },
            UiButton::InserterOverrideSlider(max),
            Button,
            bevy::ui::RelativeCursorPosition::default(),
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
                ImageNode { image: l.slider_empty.image.clone(), rect: Some(l.slider_empty.rect), ..default() },
                Pickable::IGNORE,
            ));
            if ins.stack_override.is_some() {
                bar.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        width: Val::Percent(fraction * 100.0),
                        height: Val::Px(12.0),
                        ..default()
                    },
                    crate::gui_skin::node_image(&l.slider_full),
                    Pickable::IGNORE,
                ));
            }
            bar.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(fraction * 140.0),
                    width: Val::Px(20.0),
                    height: Val::Px(12.0),
                    ..default()
                },
                ImageNode { image: l.slider_handle.image.clone(), rect: Some(l.slider_handle.rect), ..default() },
                Pickable::IGNORE,
            ));
        });
        r.spawn((
            Node {
                width: Val::Px(80.0),
                height: Val::Px(28.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            crate::gui_skin::node_image(&l.textbox),
        ))
        .with_children(|t| ctx.text(t, value.to_string(), 14.0, Color::BLACK));
    });
}

/// An empty cell of a slot pane (`deep_slots_scroll_pane`'s tiled background).
fn empty_cell(g: &mut ChildSpawnerCommands) {
    // As measured from the game: the cell is the pane's own colour, with only a lighter
    // top edge, a darker bottom edge and faint sides.
    g.spawn(Node {
        width: Val::Px(SLOT_PX),
        height: Val::Px(SLOT_PX),
        padding: UiRect::all(Val::Px(4.0)),
        ..default()
    })
    .with_children(|c| {
        c.spawn((
            Node { width: Val::Px(32.0), height: Val::Px(32.0), border: UiRect::all(Val::Px(1.0)), ..default() },
            BorderColor {
                top: Color::srgb_u8(56, 56, 56),
                bottom: Color::srgb_u8(27, 27, 27),
                left: Color::srgb_u8(35, 35, 35),
                right: Color::srgb_u8(35, 35, 35),
            },
            Pickable::IGNORE,
        ));
    });
}

/// The title bar buttons of an entity window: the circuit network button for entities
/// with circuit connectors, the logistic network button for those that can also be
/// controlled by a logistic network (as in the game), then close.
fn network_buttons(sim: &Sim, data: &Data, id: EntityId) -> Vec<&'static str> {
    // Windows with the character's inventory start with its search button.
    let mut out = if shows_inventory(sim, id) { vec!["search"] } else { Vec::new() };
    if let Some(e) = sim.0.entity(id) {
        let proto = sim.0.prototypes().entity(e.proto);
        let raw = data.0.prototype(&proto.kind, &proto.name);
        if raw.get("circuit_wire_max_distance").as_f64().is_some_and(|d| d > 0.0) {
            out.push("circuit_network_panel");
            let logistic = [
                "transport-belt",
                "inserter",
                "assembling-machine",
                "furnace",
                "mining-drill",
                "pump",
                "offshore-pump",
                "lamp",
            ];
            if logistic.contains(&proto.kind.as_str()) {
                out.push("logistic_network_panel_white");
            }
        }
    }
    out.push("close");
    out
}

/// Whether an entity's window includes the character's inventory: only for entities
/// with item slots.
fn shows_inventory(sim: &Sim, id: EntityId) -> bool {
    match sim.0.entity(id).map(|e| &e.state) {
        Some(EntityState::Belt | EntityState::Pole) => false,
        Some(EntityState::Fluid(f)) => f.energy.burner().is_some(),
        _ => true,
    }
}

/// What an entity window shows apart from its progress bars.
fn structure_sig(sim: &Sim, id: EntityId) -> String {
    let Some(e) = sim.0.entity(id) else { return String::new() };
    let burner = |energy: &factorio_sim::energy::EnergyState| {
        energy.burner().map(|b| format!("{:?}{:?}{:?}", b.fuel, b.burnt, b.currently_burning.is_some()))
    };
    let state = match &e.state {
        EntityState::Container(inv) => format!("{inv:?}"),
        EntityState::Drill(d) => format!("{:?}{:?}", burner(&d.energy), d.output),
        EntityState::Crafter(c) => format!("{:?}{:?}{:?}{:?}", c.recipe, c.input, c.output, burner(&c.energy)),
        EntityState::Inserter(i) => format!(
            "{:?}{:?}{:?}{}{}{:?}{}",
            i.hand,
            burner(&i.energy),
            i.filters,
            i.use_filters,
            i.blacklist,
            i.stack_override,
            factorio_sim::machines::hand_size(&sim.0, sim.0.prototypes().entity(e.proto))
        ),
        EntityState::Lab(l) => format!("{:?}{:?}", l.input, sim.0.research().current()),
        // Fluid amounts, power figures and graphs: refreshed once a second.
        EntityState::Fluid(f) => format!("{:?}{}", burner(&f.energy), sim.0.tick() / 60),
        EntityState::Pole => format!("{}", sim.0.tick() / 60),
        other => format!("{other:?}"),
    };
    format!("{}|{state}", status(sim, id).0)
}

/// A value shown by a bar that updates every frame without rebuilding the window.
#[derive(Component, Clone, Copy, Debug)]
enum Live {
    /// The opened machine's crafting progress.
    Craft,
    /// Fuel left in the item the opened entity is burning.
    FuelLeft,
    /// The opened drill's mining progress.
    Drill,
    /// The current research.
    Research,
    /// Durability left in the opened lab's pack in this slot.
    LabPack(u16),
    /// The opened pole's network satisfaction.
    Satisfaction,
    /// The character's mining progress.
    Mining,
}

/// The fill of a bar showing a [`Live`] value.
#[derive(Component)]
struct LiveFill(Live);

/// The percentage text of a bar showing a [`Live`] value.
#[derive(Component)]
struct LiveText(Live);

fn live_value(sim: &Sim, live: Live) -> f64 {
    let db = sim.0.prototypes();
    let r = sim.0.research();
    if let Live::Research = live {
        return r.current().map_or(0.0, |t| r.progress_fraction(db, t).to_f64_lossy());
    }
    if let Live::Mining = live {
        return mining_fraction(sim).unwrap_or(0.0);
    }
    let Some(e) = opened(sim).and_then(|id| sim.0.entity(id)) else { return 0.0 };
    let proto = db.entity(e.proto);
    let fuel_left = |energy: &factorio_sim::energy::EnergyState| {
        energy
            .burner()
            .and_then(|b| {
                let f = db.item(b.currently_burning?).fuel.as_ref()?;
                Some(b.remaining.to_f64_lossy() / f.value.to_f64_lossy().max(1e-9))
            })
            .unwrap_or(0.0)
    };
    let v = match (live, &e.state) {
        (Live::Craft, EntityState::Crafter(c)) => {
            let ticks = c.recipe.map(|r| db.recipe(r).ticks().to_f64_lossy()).unwrap_or(1.0);
            c.progress.to_f64_lossy() / ticks
        }
        (Live::Drill, EntityState::Drill(d)) => {
            let ticks = factorio_sim::machines::drill_resources(&sim.0, proto, e.position, e.direction)
                .first()
                .and_then(|t| sim.0.surface.resource(*t))
                .and_then(|r| db.entity(r.proto).minable.as_ref())
                .map(|m| m.mining_ticks.to_f64_lossy())
                .unwrap_or(1.0);
            d.progress.to_f64_lossy() / ticks
        }
        (Live::FuelLeft, EntityState::Crafter(c)) => fuel_left(&c.energy),
        (Live::FuelLeft, EntityState::Drill(d)) => fuel_left(&d.energy),
        (Live::FuelLeft, EntityState::Inserter(i)) => fuel_left(&i.energy),
        (Live::FuelLeft, EntityState::Fluid(f)) => fuel_left(&f.energy),
        (Live::FuelLeft, EntityState::Lab(l)) => fuel_left(&l.energy),
        (Live::LabPack(i), EntityState::Lab(l)) => l.opened_fraction(i as usize).to_f64_lossy(),
        (Live::Satisfaction, EntityState::Pole) => sim
            .0
            .power
            .electric_network_of
            .get(&opened(sim).unwrap())
            .map_or(0.0, |n| sim.0.power.electric_networks[*n].satisfaction().to_f64_lossy()),
        _ => 0.0,
    };
    v.clamp(0.0, 1.0)
}

/// The mining bar's fill: the game's orange, bright at the top and dark at the bottom,
/// the same across its whole length (measured from a screenshot at 125 %, 16 rows).
/// One texel wide, so stretching it never darkens the ends.
fn mining_gradient() -> Image {
    const ROWS: [[u8; 3]; 16] = [
        [249, 168, 56],
        [244, 164, 55],
        [234, 158, 53],
        [219, 148, 49],
        [204, 137, 46],
        [188, 127, 42],
        [173, 117, 39],
        [157, 106, 35],
        [141, 95, 32],
        [125, 85, 28],
        [110, 74, 25],
        [95, 64, 21],
        [76, 51, 17],
        [59, 39, 13],
        [47, 31, 10],
        [39, 27, 9],
    ];
    let mut image = Image::new(
        bevy::render::render_resource::Extent3d { width: 1, height: 16, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        ROWS.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

/// How far the character is through mining what it is mining, if anything.
fn mining_fraction(sim: &Sim) -> Option<f64> {
    let db = sim.0.prototypes();
    let m = character(sim)?.mining.as_ref()?;
    let proto = match m.target {
        factorio_sim::player::MiningTarget::Entity(id) => sim.0.entity(id)?.proto,
        factorio_sim::player::MiningTarget::Resource(t) => sim.0.surface.resource(t)?.proto,
    };
    let ticks = db.entity(proto).minable.as_ref()?.mining_ticks.to_f64_lossy();
    Some((m.progress.to_f64_lossy() / ticks.max(1e-9)).clamp(0.0, 1.0))
}

/// The mining bar on top of the quickbar while the character mines, as in the game.
fn mining_bar(sim: Res<Sim>, ui: Res<UiState>, mut q: Query<&mut Node, With<MiningBar>>) {
    let show = mining_fraction(&sim).is_some() && !ui.tech_open;
    for mut n in &mut q {
        let d = if show { Display::Flex } else { Display::None };
        if n.display != d {
            n.display = d;
        }
    }
}

/// Updates the [`Live`] bars every frame.
fn live_bars(
    sim: Res<Sim>,
    mut fills: Query<(&LiveFill, &mut Node)>,
    mut texts: Query<(&LiveText, &mut Text, &mut TextColor)>,
) {
    for (f, mut node) in &mut fills {
        let w = Val::Percent((live_value(&sim, f.0) * 100.0) as f32);
        if node.width != w {
            node.width = w;
        }
    }
    for (t, mut text, mut color) in &mut texts {
        let v = live_value(&sim, t.0);
        let s = format!("{:.0}%", v * 100.0);
        if text.0 != s {
            text.0 = s;
            // The percentage turns dark once the bar is filled under it.
            color.0 = if v > 0.9 { Color::BLACK } else { Color::WHITE };
        }
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
        EntityState::Belt => ("Working", green),
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
                    .with_children(|c| progress_bar_live(c, left, looks().burning_bar_color, Some(Live::FuelLeft)));
            });
        }
    };
    match &e.state {
        EntityState::Container(inv) => {
            // As in the game: the slots, then the limit button in the next cell, then
            // empty cells to the end of the row. Click the button, then a slot, to stop
            // automatic filling from that slot on; right click removes the limit.
            let l = looks();
            let look = if local.limit_mode { &l.yellow_slot } else { &l.red_slot };
            grid(p, 10, |g| {
                for i in 0..inv.len() {
                    ctx.inventory_slot(g, inv, i, &|i| SlotRef::Opened(EntityInventory::Main, i as u16));
                }
                g.spawn((
                    Node {
                        width: Val::Px(SLOT_PX),
                        height: Val::Px(SLOT_PX),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    crate::gui_skin::node_image(&look.default),
                    look.clone(),
                    UiButton::ContainerLimit,
                    Button,
                    Tip::Text("Limit: click, then click a slot. Right click removes the limit.".into()),
                ))
                .with_children(|b| ctx.utility(b, "set_bar_slot", 32.0));
                for _ in (inv.len() + 1)..(inv.len() + 1).div_ceil(10) * 10 {
                    empty_cell(g);
                }
            });
        }
        EntityState::Drill(d) => {
            let ticks = factorio_sim::machines::drill_resources(&sim.0, proto, e.position, e.direction)
                .first()
                .and_then(|t| sim.0.surface.resource(*t))
                .and_then(|r| db.entity(r.proto).minable.as_ref())
                .map(|m| m.mining_ticks.to_f64_lossy())
                .unwrap_or(1.0);
            p.spawn(Node { flex_direction: FlexDirection::Row, ..default() }).with_children(|r| {
                production_bar(r, ctx, (d.progress.to_f64_lossy() / ticks).clamp(0.0, 1.0), Live::Drill)
            });
            module_row(p, ctx, proto);
            if d.energy.burner().is_some() {
                separator(p);
            }
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
                        TextFont { font: ctx.fonts.regular.clone(), font_size: 15.0, ..default() },
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
                production_bar(r, ctx, fraction, Live::Craft);
                let n = c.output.slots().len().max(1);
                ctx.inventory(r, &c.output, n, |i| SlotRef::Opened(EntityInventory::Output, i as u16));
            });
            module_row(p, ctx, proto);
            if c.energy.burner().is_some() {
                separator(p);
            }
            fuel_slots(p, ctx, &c.energy);
        }
        EntityState::Inserter(i) => {
            // The stack in the hand.
            slot_row(p, ctx, 1, |r, ctx| match i.hand {
                Some(h) => ctx.slot(r, Some(h.item), Some(h.count), SLOT, None, None),
                None => ghost_slot(r, ctx, "empty_inserter_hand_slot"),
            });
            // As in the game: hand, fuel, filters, stack size override.
            if i.energy.burner().is_some() {
                separator(p);
            }
            fuel_slots(p, ctx, &i.energy);
            if !i.filters.is_empty() {
                separator(p);
                inserter_filters(p, ctx, i);
            }
            separator(p);
            inserter_stack_override(p, ctx, sim, proto, i);
        }
        // Fluid contents and power output are shown in the hover info panel, not here.
        EntityState::Fluid(f) => fuel_slots(p, ctx, &f.energy),
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

/// Width of a column of the electric network window (the game's `production_graph_width`).
const NETWORK_COL_W: f32 = 556.0;

/// A bar with its text inside on the right (the game's network and production bars).
fn value_bar(p: &mut ChildSpawnerCommands, ctx: &Ctx, fraction: f64, color: Color, text: String, live: Option<Live>) {
    let l = looks();
    p.spawn((
        Node {
            width: Val::Percent(100.0),
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
        bar.color = color;
        let mut fill = b.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent((fraction.clamp(0.0, 1.0) * 100.0) as f32),
                height: Val::Percent(100.0),
                ..default()
            },
            bar,
        ));
        if let Some(live) = live {
            fill.insert(LiveFill(live));
        }
        b.spawn((
            Text::new(text),
            TextFont { font: ctx.fonts.regular.clone(), font_size: 14.0, ..default() },
            TextColor(Color::WHITE),
        ));
    });
}

/// A titled section of the network window: a subheader with the title (and optional
/// reset button) over its contents.
fn network_section(
    p: &mut ChildSpawnerCommands,
    ctx: &mut Ctx,
    title: &str,
    reset: bool,
    f: impl FnOnce(&mut ChildSpawnerCommands, &mut Ctx),
) {
    p.spawn(Node { width: Val::Px(NETWORK_COL_W), flex_direction: FlexDirection::Column, ..default() }).with_children(
        |col| {
            crate::gui_skin::backdrop(col, &looks().deep_in_shallow);
            col.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                height: Val::Px(36.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(0.0)),
                ..default()
            })
            .with_children(|h| {
                h.spawn((
                    Text::new(title),
                    TextFont { font: ctx.fonts.bold.clone(), font_size: 15.0, ..default() },
                    TextColor(looks().title_color),
                    Node { flex_grow: 1.0, ..default() },
                ));
                if reset {
                    let l = looks();
                    h.spawn((
                        Node {
                            width: Val::Px(28.0),
                            height: Val::Px(28.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        crate::gui_skin::node_image(&l.red_button.default),
                        l.red_button.clone(),
                        Button,
                        Tip::Text("Reset".into()),
                    ))
                    .with_children(|b| ctx.utility(b, "reset", 24.0));
                }
            });
            f(col, ctx);
        },
    );
}

fn network_window(
    p: &mut ChildSpawnerCommands,
    ctx: &mut Ctx,
    sim: &Sim,
    id: EntityId,
    chart: &crate::chart::Chart,
    images: &mut Assets<Image>,
) {
    use crate::chart::{PALETTE, power_text};
    let db = ctx.db;
    let Some(n) = sim.0.power.electric_network_of.get(&id).map(|n| &sim.0.power.electric_networks[*n]) else {
        ctx.text(p, "Not connected", 14.0, TEXT);
        return;
    };
    let watts = |e: factorio_sim::Fixed| power_text(e.to_f64_lossy());
    // Top: satisfaction, production and accumulator charge.
    p.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(12.0), ..default() }).with_children(|row| {
        let sat = n.satisfaction().to_f64_lossy();
        network_section(row, ctx, "Satisfaction", false, |c, ctx| {
            value_bar(
                c,
                ctx,
                sat,
                looks().production_bar_color,
                format!("{} / {}", watts(n.production), watts(n.demand)),
                Some(Live::Satisfaction),
            );
        });
        let cap = n.capacity.to_f64_lossy();
        let prod = if cap > 0.0 { n.production.to_f64_lossy() / cap } else { 0.0 };
        network_section(row, ctx, "Production", false, |c, ctx| {
            value_bar(
                c,
                ctx,
                prod,
                looks().production_bar_color,
                format!("{} / {}", watts(n.production), watts(n.capacity)),
                None,
            );
        });
        network_section(row, ctx, "Accumulator charge", false, |c, ctx| {
            value_bar(c, ctx, 0.0, looks().production_bar_color, "0 W / 0 W".into(), None);
        });
    });
    // Time ranges. Only the first three are recorded so far.
    let font = ctx.fonts.regular.clone();
    p.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(4.0), ..default() }).with_children(|row| {
        for (i, name) in ["5s", "1m", "10m", "1h", "10h", "50h", "250h", "1000h"].iter().enumerate() {
            let l = looks();
            let look = if chart.range == i { &l.yellow_slot } else { &l.button };
            let mut b = row.spawn((
                Node {
                    flex_grow: 1.0,
                    height: Val::Px(28.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                crate::gui_skin::node_image(&look.default),
                look.clone(),
            ));
            if i < 3 {
                b.insert((UiButton::ChartRange(i), Button));
            }
            b.with_children(|b| {
                b.spawn((
                    Text::new(*name),
                    TextFont { font: font.clone(), font_size: 14.0, ..default() },
                    TextColor(Color::BLACK),
                ));
            });
        }
    });
    let Some(stats) = sim.0.power.stats_for(id) else { return };
    crate::chart::draw(images, chart, stats);
    let last = stats.series[chart.range].samples.back().cloned().unwrap_or_default();
    // Entity counts per type in this network.
    let mut counts: std::collections::BTreeMap<factorio_sim::proto::EntityProtoId, u32> = Default::default();
    for e in n.consumers.iter().chain(&n.generators) {
        if let Some(e) = sim.0.entity(*e) {
            *counts.entry(e.proto).or_default() += 1;
        }
    }
    let rows = |c: &mut ChildSpawnerCommands,
                ctx: &mut Ctx,
                entries: Vec<(factorio_sim::proto::EntityProtoId, f64, [u8; 3])>| {
        c.spawn(Node {
            display: Display::Grid,
            grid_template_columns: RepeatedGridTrack::flex(2, 1.0),
            column_gap: Val::Px(12.0),
            padding: UiRect::all(Val::Px(12.0)),
            min_height: Val::Px(200.0),
            align_content: AlignContent::Start,
            ..default()
        })
        .with_children(|g| {
            for (proto, v, color) in entries {
                g.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    height: Val::Px(32.0),
                    ..default()
                })
                .with_children(|r| {
                    r.spawn((
                        Node { width: Val::Px(4.0), height: Val::Px(24.0), ..default() },
                        BackgroundColor(Color::srgb_u8(color[0], color[1], color[2])),
                    ));
                    let name = db.entity(proto).name.clone();
                    let d = ctx.data.0.clone();
                    if let Some(icon) = ctx.sprites.get(ctx.assets, ctx.data, &format!("eicon:{name}"), || {
                        factorio_data::sprite::item_icon(&d, &name)
                    }) {
                        let s = &icon.sprite;
                        r.spawn((
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
                            Node { width: Val::Px(24.0), height: Val::Px(24.0), ..default() },
                        ));
                    }
                    ctx.text(r, format!("x{}", counts.get(&proto).copied().unwrap_or(0)), 13.0, TEXT);
                    r.spawn(Node { flex_grow: 1.0, ..default() });
                    ctx.text(r, power_text(v), 13.0, Color::WHITE);
                });
            }
        });
    };
    let graph = |c: &mut ChildSpawnerCommands, image: &Handle<Image>| {
        c.spawn(Node { padding: UiRect::axes(Val::Px(18.0), Val::Px(0.0)), ..default() }).with_children(|g| {
            g.spawn((
                ImageNode::new(image.clone()),
                Node {
                    width: Val::Px(crate::chart::WIDTH as f32),
                    height: Val::Px(crate::chart::HEIGHT as f32),
                    ..default()
                },
            ));
        });
    };
    p.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(12.0), ..default() }).with_children(|row| {
        let consumers: Vec<_> = crate::chart::series_order(stats, chart.range)
            .into_iter()
            .enumerate()
            .map(|(k, proto)| (proto, last.consumption.get(&proto).map_or(0.0, |v| v.to_f64_lossy()), PALETTE[k]))
            .collect();
        network_section(row, ctx, "Consumption", true, |c, ctx| {
            graph(c, &chart.image);
            rows(c, ctx, consumers);
        });
        let producers: Vec<_> = crate::chart::production_order(stats, chart.range)
            .into_iter()
            .enumerate()
            .map(|(k, proto)| (proto, last.production.get(&proto).map_or(0.0, |v| v.to_f64_lossy()), PALETTE[k]))
            .collect();
        network_section(row, ctx, "Production", true, |c, ctx| {
            graph(c, &chart.production);
            rows(c, ctx, producers);
        });
        network_section(row, ctx, "Accumulator charge", false, |c, _| {
            graph(c, &chart.accumulator);
        });
    });
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
    mut misses: Local<u8>,
    scale: Res<UiScale>,
) {
    let tip = hovered.iter().find(|(i, _)| **i != Interaction::None).map(|(_, t)| t.clone()).or_else(|| test_tip(&sim));
    let Some(tip) = tip else {
        // A rebuilt window's buttons only learn they are hovered a frame later; keep the
        // tooltip through that frame instead of blinking it.
        *misses = misses.saturating_add(1);
        if *misses > 2 {
            root.1.display = Display::None;
            last.clear();
        }
        return;
    };
    *misses = 0;
    if let Some(c) = window.cursor_position().or(test_tip(&sim).map(|_| Vec2::new(40.0, 120.0))) {
        // UI lengths are scaled by the GUI scale; the cursor is in window pixels.
        let c = c / scale.0;
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
    let mut ctx = Ctx {
        sprites: &mut sprites,
        assets: &assets,
        data: &data,
        fonts: &fonts,
        db,
        research: sim.0.research(),
        hovered: None,
    };
    let root = root.0;
    commands.entity(root).despawn_related::<Children>();
    commands.entity(root).with_children(|p| tips::tooltip_contents(p, &mut ctx, &names, &sim, &tip));
}

/// `FACTORIO_REWRITE_TIP=item:<name>|recipe:<name>|tech:<name>` shows that tooltip
/// without hovering, for screenshots.
fn test_tip(sim: &Sim) -> Option<Tip> {
    let v = std::env::var("FACTORIO_REWRITE_TIP").ok()?;
    let (kind, name) = v.split_once(':')?;
    let db = sim.0.prototypes();
    match kind {
        "item" => db.item_id(name).map(Tip::Item),
        "recipe" => db.recipe_id(name).map(Tip::Recipe),
        "tech" => db.technology_id(name).map(Tip::Tech),
        _ => None,
    }
}

/// The info panel for the entity under the mouse.
#[allow(clippy::too_many_arguments)]
fn entity_info(
    mut commands: Commands,
    sim: Res<Sim>,
    data: Res<Data>,
    assets: Res<AssetServer>,
    fonts: Res<Fonts>,
    names: Res<Names>,
    ui: Res<UiState>,
    mouse: Res<MouseWorld>,
    mut sprites: ResMut<Sprites>,
    mut root: Single<(Entity, &mut Node), With<EntityInfoRoot>>,
    mut last: Local<String>,
) {
    // `FACTORIO_REWRITE_INFO=1`: the opened entity's info, for screenshots.
    let test = std::env::var_os("FACTORIO_REWRITE_INFO").and_then(|_| opened(&sim));
    let id = mouse
        .0
        .filter(|_| !ui.pointer_over_ui && !ui.tech_open)
        .and_then(|p| sim.0.entity_at(p))
        .or(test)
        .filter(|id| sim.0.entity(*id).is_some_and(|e| !matches!(e.state, EntityState::Static { .. })));
    let Some(id) = id else {
        root.1.display = Display::None;
        last.clear();
        return;
    };
    root.1.display = Display::Flex;
    // Refreshed when the entity's state changes, at most a few times a second.
    let sig = format!("{id:?}|{}|{}", structure_sig(&sim, id), sim.0.tick() / 15);
    if *last == sig {
        return;
    }
    *last = sig;
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
    commands.entity(root.0).despawn_related::<Children>();
    commands.entity(root.0).with_children(|p| tips::entity_info(p, &mut ctx, &names, &sim, id));
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
    scale: Res<UiScale>,
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
    let c = c / scale.0;
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
