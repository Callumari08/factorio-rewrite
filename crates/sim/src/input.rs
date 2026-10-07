//! Player inputs: the only way anything outside the simulation can change it.
//!
//! In lockstep multiplayer, peers exchange these per tick and every peer applies the
//! same list in the same order, so they must be plain data with a total order.

use crate::map::{Direction, MapPosition};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InputAction {
    /// Place an entity by prototype name (dev-only until the player/inventory exists).
    DebugPlaceEntity { prototype: String, position: MapPosition, direction: Direction },
    /// Remove the entity occupying the given position, if any.
    DebugRemoveEntity { position: MapPosition },
}

/// Inputs from one player for one tick.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerInput {
    pub player: u16,
    pub action: InputAction,
}
