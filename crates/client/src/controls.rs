//! Keyboard and mouse → simulation inputs. Mirrors Factorio's default controls where
//! possible: WASD walk, left click build/open, right click hold mine, R rotate, Q clear
//! cursor/pick, F pick up, E inventory.

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use factorio_sim::input::InputAction;
use factorio_sim::map::{Direction, MapPosition, TilePosition};

use crate::{Cursor, LOCAL_PLAYER, PendingInputs, Sim, world_to_map};

pub struct ControlsPlugin;

impl Plugin for ControlsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MouseWorld>()
            .init_resource::<UiState>()
            .add_systems(Startup, open_from_env)
            .add_systems(Update, open_later)
            .add_systems(Update, (update_mouse_world, keyboard, mouse, zoom).chain());
    }
}

/// The map position under the mouse cursor.
#[derive(Resource, Default)]
pub struct MouseWorld(pub Option<MapPosition>);

#[derive(Resource, Default)]
pub struct UiState {
    /// The character window (E). Entity windows are opened through the simulation.
    pub inventory_open: bool,
    /// The technology window (T).
    pub tech_open: bool,
    /// The settings panel (Escape with no window open).
    pub settings_open: bool,
    /// True while the pointer is over a UI panel.
    pub pointer_over_ui: bool,
    /// True while a search box takes the keyboard: game keys are ignored.
    pub typing: bool,
    pub status: String,
}

#[derive(Default)]
struct ControlState {
    walking: Option<Direction>,
    mining_tile: Option<TilePosition>,
    last_build_tile: Option<TilePosition>,
}

/// `FACTORIO_REWRITE_UI=1` opens the inventory (and the first machine's window) at start,
/// for screenshots.
/// An entity for [`open_from_env`] to open once the character has moved next to it.
#[derive(Resource)]
struct OpenLater(MapPosition);

/// Something for [`open_from_env`] to mine once the character has moved next to it.
#[derive(Resource)]
struct MineLater(MapPosition);

fn open_later(
    mut commands: Commands,
    later: Option<Res<OpenLater>>,
    mine: Option<Res<MineLater>>,
    mut pending: ResMut<PendingInputs>,
    mut frames: Local<u32>,
) {
    *frames += 1;
    if *frames != 10 {
        return;
    }
    if let Some(later) = later {
        pending.push(InputAction::OpenEntity(Some(later.0)));
        commands.remove_resource::<OpenLater>();
    }
    if let Some(m) = mine {
        pending.push(InputAction::SetMining(Some(m.0)));
        commands.remove_resource::<MineLater>();
    }
}

fn open_from_env(mut commands: Commands, sim: Res<Sim>, mut ui: ResMut<UiState>, mut pending: ResMut<PendingInputs>) {
    if std::env::var_os("FACTORIO_REWRITE_UI").is_none() {
        return;
    }
    // `FACTORIO_REWRITE_UI=tech` opens the technology window instead.
    if std::env::var("FACTORIO_REWRITE_UI").is_ok_and(|v| v == "tech") {
        ui.tech_open = true;
        return;
    }
    if std::env::var("FACTORIO_REWRITE_UI").is_ok_and(|v| v == "settings") {
        ui.settings_open = true;
        return;
    }
    ui.inventory_open = true;
    // `FACTORIO_REWRITE_UI=mine` mines the nearest rock or tree (showing the mining bar).
    if std::env::var("FACTORIO_REWRITE_UI").is_ok_and(|v| v == "mine") {
        ui.inventory_open = false;
        let near = sim
            .0
            .entities()
            .filter(|(_, e)| matches!(e.state, factorio_sim::world::EntityState::Static { .. }))
            .min_by_key(|(_, e)| (e.position.x as i64).pow(2) + (e.position.y as i64).pow(2))
            .map(|(_, e)| e.position);
        if let Some(p) = near {
            pending.push(InputAction::CheatTeleport(p.offset(0, 2 * 256)));
            commands.insert_resource(MineLater(p));
        }
        return;
    }
    // `FACTORIO_REWRITE_UI=hand` picks up the first inventory stack (showing the hand).
    if std::env::var("FACTORIO_REWRITE_UI").is_ok_and(|v| v == "hand") {
        pending.push(InputAction::ClickSlot {
            slot: factorio_sim::input::SlotRef::Character(0),
            button: factorio_sim::input::MouseButton::Left,
            shift: false,
            ctrl: false,
        });
    }
    // `FACTORIO_REWRITE_UI=power` opens a pole (the network window) instead of a machine,
    // `lab` a lab, `chest` a container, `furnace` a furnace. The character is moved next to
    // it so it is in reach.
    let want = std::env::var("FACTORIO_REWRITE_UI").unwrap_or_default();
    let at = sim
        .0
        .entities()
        .find(|(_, e)| match &e.state {
            factorio_sim::world::EntityState::Pole => want == "power",
            factorio_sim::world::EntityState::Lab(_) => want == "lab",
            factorio_sim::world::EntityState::Container(_) => want == "chest",
            factorio_sim::world::EntityState::Drill(_) => want == "drill",
            factorio_sim::world::EntityState::Inserter(_) => want == "inserter",
            factorio_sim::world::EntityState::Belt => want == "belt",
            factorio_sim::world::EntityState::Fluid(_) => want == "fluid",
            factorio_sim::world::EntityState::Crafter(c) => {
                if want == "furnace" {
                    c.furnace
                } else {
                    !c.furnace && want == "1"
                }
            }
            _ => false,
        })
        .map(|(_, e)| e.position);
    if let Some(p) = at {
        pending.push(InputAction::CheatTeleport(p.offset(0, 3 * 256)));
        commands.insert_resource(OpenLater(p));
    }
}

fn update_mouse_world(
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform)>,
    mut mouse: ResMut<MouseWorld>,
) {
    let (camera, transform) = *camera;
    mouse.0 = window.cursor_position().and_then(|c| camera.viewport_to_world_2d(transform, c).ok()).map(world_to_map);
}

fn held_item(sim: &Sim) -> Option<factorio_sim::proto::ItemId> {
    sim.0.player(LOCAL_PLAYER).and_then(|p| p.character.as_ref()).and_then(|c| c.cursor).map(|c| c.item)
}

fn keyboard(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut focus: MessageReader<bevy::window::WindowFocused>,
    sim: Res<Sim>,
    mouse: Res<MouseWorld>,
    mut cursor: ResMut<Cursor>,
    mut ui: ResMut<UiState>,
    mut pending: ResMut<PendingInputs>,
    mut state: Local<ControlState>,
    mut frames: Local<u32>,
) {
    // When the window loses focus key releases are never seen, so forget held keys;
    // otherwise the character keeps walking (the game only walks while keys are held).
    if focus.read().any(|f| !f.focused) {
        keys.reset_all();
    }
    if ui.typing {
        // The search box has the keyboard; stop walking.
        if state.walking.is_some() {
            state.walking = None;
            pending.push(InputAction::SetWalking(None));
        }
        return;
    }
    let (mut dx, mut dy) = (0, 0);
    if keys.pressed(KeyCode::KeyW) {
        dy -= 1;
    }
    if keys.pressed(KeyCode::KeyS) {
        dy += 1;
    }
    if keys.pressed(KeyCode::KeyA) {
        dx -= 1;
    }
    if keys.pressed(KeyCode::KeyD) {
        dx += 1;
    }
    let walking = match (dx, dy) {
        (0, -1) => Some(Direction(0)),
        (1, -1) => Some(Direction(2)),
        (1, 0) => Some(Direction(4)),
        (1, 1) => Some(Direction(6)),
        (0, 1) => Some(Direction(8)),
        (-1, 1) => Some(Direction(10)),
        (-1, 0) => Some(Direction(12)),
        (-1, -1) => Some(Direction(14)),
        _ => None,
    };
    // Re-send now and then too, so the simulation can never drift from the keys held.
    *frames += 1;
    let sim_walking = sim.0.player(LOCAL_PLAYER).and_then(|p| p.character.as_ref()).and_then(|c| c.walking);
    if walking != state.walking || (frames.is_multiple_of(15) && sim_walking != walking) {
        state.walking = walking;
        pending.push(InputAction::SetWalking(walking));
    }

    let opened = sim.0.player(LOCAL_PLAYER).and_then(|p| p.opened).is_some();
    if keys.just_pressed(KeyCode::KeyT) {
        ui.tech_open = !ui.tech_open;
        if ui.tech_open {
            ui.inventory_open = false;
            pending.push(InputAction::OpenEntity(None));
        }
    }
    if keys.just_pressed(KeyCode::KeyE) {
        ui.tech_open = false;
        if opened || ui.inventory_open {
            ui.inventory_open = false;
            pending.push(InputAction::OpenEntity(None));
        } else {
            ui.inventory_open = true;
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        // Escape closes what is open; with nothing open it shows the settings.
        if !(opened || ui.inventory_open || ui.tech_open) {
            ui.settings_open = !ui.settings_open;
        }
        ui.inventory_open = false;
        ui.tech_open = false;
        pending.push(InputAction::OpenEntity(None));
    }
    let held = held_item(&sim);
    if keys.just_pressed(KeyCode::KeyR) {
        let reverse = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if held.is_some_and(|i| sim.0.prototypes().item(i).place_result.is_some()) {
            cursor.direction = if reverse { cursor.direction.rotate_ccw() } else { cursor.direction.rotate_cw() };
        } else if let Some(p) = mouse.0 {
            pending.push(InputAction::Rotate { position: p, reverse });
        }
    }
    if keys.just_pressed(KeyCode::KeyQ) {
        if held.is_some() {
            pending.push(InputAction::ClearCursor);
        } else if let Some(id) = mouse.0.and_then(|p| sim.0.entity_at(p)) {
            // Pipette: hold the item that builds the hovered entity, if we have one.
            let e = sim.0.entity(id).unwrap();
            if let Some(item) = sim.0.prototypes().item_to_place(e.proto) {
                pending.push(InputAction::PickItem(item));
                cursor.direction = e.direction;
            }
        }
    }
    // Quickbar: 1-0 pick the item in the first row.
    let digits = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
        KeyCode::Digit0,
    ];
    for (i, k) in digits.iter().enumerate() {
        if keys.just_pressed(*k)
            && let Some(item) = sim.0.player(LOCAL_PLAYER).and_then(|p| p.quickbar[i])
        {
            pending.push(InputAction::PickItem(item));
        }
    }
    if keys.pressed(KeyCode::KeyF) {
        pending.push(InputAction::PickupItems);
    }
    if keys.just_pressed(KeyCode::F2) {
        let on = !sim.0.player(LOCAL_PLAYER).is_some_and(|p| p.cheat_mode);
        pending.push(InputAction::SetCheatMode(on));
        ui.status = if on { "Cheat mode on: crafting is instant and free" } else { "Cheat mode off" }.into();
    }
    if keys.just_pressed(KeyCode::F3) {
        pending.push(InputAction::CheatResearchAll);
        ui.status = "Researched every technology".into();
    }
    if keys.just_pressed(KeyCode::F1) {
        pending.push(InputAction::CheatAllItems);
        ui.status = "Added a stack of every item (overflow in chests next to you)".into();
    }
}

fn mouse(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    sim: Res<Sim>,
    mouse: Res<MouseWorld>,
    cursor: Res<Cursor>,
    mut ui: ResMut<UiState>,
    mut pending: ResMut<PendingInputs>,
    mut state: Local<ControlState>,
) {
    let Some(at) = mouse.0 else { return };
    let tile = at.tile();
    let over_ui = ui.pointer_over_ui;
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let held = held_item(&sim);
    let buildable = held.filter(|i| sim.0.prototypes().item(*i).place_result.is_some());

    if !over_ui && ctrl {
        // Ctrl+click: fast transfer (right button: half).
        if buttons.just_pressed(MouseButton::Left) {
            pending.push(InputAction::FastTransfer { position: at, half: false });
        } else if buttons.just_pressed(MouseButton::Right) {
            pending.push(InputAction::FastTransfer { position: at, half: true });
        }
        return;
    }

    // Left: build the held item (dragging builds along the way), or open an entity.
    if buttons.pressed(MouseButton::Left) && !over_ui {
        if let Some(item) = buildable {
            if buttons.just_pressed(MouseButton::Left) || state.last_build_tile != Some(tile) {
                pending.push(InputAction::Build { item, position: at, direction: cursor.direction });
                state.last_build_tile = Some(tile);
            }
        } else if buttons.just_pressed(MouseButton::Left) {
            let target =
                sim.0.entity_at(at).filter(|id| factorio_sim::cursor::has_window(sim.0.prototypes(), &sim.0, *id));
            pending.push(InputAction::OpenEntity(target.map(|_| at)));
            if target.is_none() {
                ui.inventory_open = false;
            }
        }
    }
    if buttons.just_released(MouseButton::Left) {
        state.last_build_tile = None;
    }

    // Right (held): mine what is under the cursor, also while holding an item, as in the
    // game. Clearing the cursor is Q.
    if buttons.pressed(MouseButton::Right) && !over_ui {
        if state.mining_tile != Some(tile) {
            state.mining_tile = Some(tile);
            pending.push(InputAction::SetMining(Some(at)));
        }
    } else if state.mining_tile.is_some() {
        state.mining_tile = None;
        pending.push(InputAction::SetMining(None));
    }
}

/// The game's zoom range in the world view: in to 3x, out to 0.3x, and no further out
/// than shows 200 tiles along the window's longest side (the game limits zoom by the
/// longest screen dimension). Our camera scale is 1 / zoom.
pub fn zoom_limits(window: &Window) -> (f32, f32) {
    let longest = window.width().max(window.height());
    let min_zoom = (longest / (32.0 * 200.0)).max(0.3);
    (1.0 / 3.0, 1.0 / min_zoom)
}

fn zoom(
    scroll: Res<AccumulatedMouseScroll>,
    ui: Res<UiState>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    mut cam: Query<&mut Projection, With<Camera2d>>,
) {
    let Ok(mut proj) = cam.single_mut() else { return };
    let (lo, hi) = zoom_limits(&window);
    if let Projection::Orthographic(o) = proj.as_mut() {
        if scroll.delta.y != 0.0 && !ui.pointer_over_ui {
            o.scale *= 1.0 - scroll.delta.y.signum() * 0.1;
        }
        // Also keeps the zoom in range when the window is resized.
        o.scale = o.scale.clamp(lo, hi);
    }
}
