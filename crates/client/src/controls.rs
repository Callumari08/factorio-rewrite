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
            .add_systems(Update, (update_mouse_world, keyboard, mouse, zoom).chain());
    }
}

/// The map position under the mouse cursor.
#[derive(Resource, Default)]
pub struct MouseWorld(pub Option<MapPosition>);

#[derive(Resource, Default)]
pub struct UiState {
    pub inventory_open: bool,
    /// Position of the entity whose window is open.
    pub opened: Option<MapPosition>,
    /// True while the pointer is over a UI panel.
    pub pointer_over_ui: bool,
    pub status: String,
}

#[derive(Default)]
struct ControlState {
    walking: Option<Direction>,
    mining_tile: Option<TilePosition>,
    last_build_tile: Option<TilePosition>,
}

fn update_mouse_world(
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform)>,
    mut mouse: ResMut<MouseWorld>,
) {
    let (camera, transform) = *camera;
    mouse.0 = window.cursor_position().and_then(|c| camera.viewport_to_world_2d(transform, c).ok()).map(world_to_map);
}

fn sandbox_kit(sim: &Sim, pending: &mut PendingInputs) {
    let db = sim.0.prototypes();
    for (name, count) in [
        ("transport-belt", 400),
        ("underground-belt", 20),
        ("splitter", 10),
        ("burner-mining-drill", 10),
        ("electric-mining-drill", 10),
        ("stone-furnace", 20),
        ("burner-inserter", 20),
        ("inserter", 50),
        ("wooden-chest", 10),
        ("iron-chest", 10),
        ("coal", 200),
        ("iron-plate", 200),
        ("copper-plate", 200),
        ("offshore-pump", 2),
        ("boiler", 4),
        ("steam-engine", 8),
        ("small-electric-pole", 50),
        ("pipe", 50),
        ("pipe-to-ground", 10),
        ("assembling-machine-1", 10),
    ] {
        if let Some(item) = db.item_id(name) {
            pending.push(InputAction::CheatItems { item, count });
        }
    }
}

fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    sim: Res<Sim>,
    mouse: Res<MouseWorld>,
    mut cursor: ResMut<Cursor>,
    mut ui: ResMut<UiState>,
    mut pending: ResMut<PendingInputs>,
    mut state: Local<ControlState>,
) {
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
    if walking != state.walking {
        state.walking = walking;
        pending.push(InputAction::SetWalking(walking));
    }

    if keys.just_pressed(KeyCode::KeyE) {
        ui.inventory_open = !ui.inventory_open;
        if !ui.inventory_open {
            ui.opened = None;
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        ui.inventory_open = false;
        ui.opened = None;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        let reverse = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if cursor.item.is_some() {
            cursor.direction = if reverse { cursor.direction.rotate_ccw() } else { cursor.direction.rotate_cw() };
        } else if let Some(p) = mouse.0 {
            pending.push(InputAction::Rotate { position: p, reverse });
        }
    }
    if keys.just_pressed(KeyCode::KeyQ) {
        if cursor.item.is_some() {
            cursor.item = None;
        } else if let Some(id) = mouse.0.and_then(|p| sim.0.entity_at(p)) {
            // Pipette: hold the item that builds the hovered entity, if we have one.
            let db = sim.0.prototypes();
            let e = sim.0.entity(id).unwrap();
            let item = db.item_to_place(e.proto);
            let have = sim
                .0
                .player(LOCAL_PLAYER)
                .and_then(|p| p.character.as_ref())
                .zip(item)
                .is_some_and(|(c, i)| c.inventory.count(i) > 0);
            if have {
                cursor.item = item;
                cursor.direction = e.direction;
            }
        }
    }
    if keys.pressed(KeyCode::KeyF) {
        pending.push(InputAction::PickupItems);
    }
    if keys.just_pressed(KeyCode::F1) {
        sandbox_kit(&sim, &mut pending);
        ui.status = "Sandbox kit added to inventory".into();
    }
    // Drop the cursor item when it runs out.
    if let Some(item) = cursor.item {
        let have =
            sim.0.player(LOCAL_PLAYER).and_then(|p| p.character.as_ref()).is_some_and(|c| c.inventory.count(item) > 0);
        if !have {
            cursor.item = None;
        }
    }
}

fn mouse(
    buttons: Res<ButtonInput<MouseButton>>,
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

    // Left: build with the cursor item (dragging builds along the way), or open an entity.
    if buttons.pressed(MouseButton::Left) && !over_ui {
        if let Some(item) = cursor.item {
            let first = buttons.just_pressed(MouseButton::Left);
            if first || state.last_build_tile != Some(tile) {
                pending.push(InputAction::Build { item, position: at, direction: cursor.direction });
                state.last_build_tile = Some(tile);
            }
        } else if buttons.just_pressed(MouseButton::Left) {
            if let Some(id) = sim.0.entity_at(at) {
                ui.opened = Some(sim.0.entity(id).unwrap().position);
                ui.inventory_open = true;
            } else {
                ui.opened = None;
            }
        }
    }
    if buttons.just_released(MouseButton::Left) {
        state.last_build_tile = None;
    }

    // Right: hold to mine whatever is under the cursor.
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

fn zoom(scroll: Res<AccumulatedMouseScroll>, ui: Res<UiState>, mut cam: Query<&mut Projection, With<Camera2d>>) {
    if scroll.delta.y == 0.0 || ui.pointer_over_ui {
        return;
    }
    let Ok(mut proj) = cam.single_mut() else { return };
    if let Projection::Orthographic(o) = proj.as_mut() {
        o.scale = (o.scale * (1.0 - scroll.delta.y.signum() * 0.1)).clamp(0.2, 6.0);
    }
}
