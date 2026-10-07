//! Player inputs: the only way anything outside the simulation can change it.
//!
//! In lockstep multiplayer, peers exchange these per tick and every peer applies the
//! same list in the same order, so they must be plain data with a total order.

use crate::map::{Direction, MapPosition};
use crate::proto::{EntityProtoId, ItemId, RecipeId};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InputAction {
    /// Creates the player's character near the spawn point if it has none.
    JoinGame,
    /// Starts walking in a direction (8-way), or stops with `None`.
    SetWalking(Option<Direction>),
    /// Starts mining whatever is at the position (entity first, then resource), or stops.
    SetMining(Option<MapPosition>),
    /// Builds the item's place result from the character's inventory.
    Build { item: ItemId, position: MapPosition, direction: Direction },
    /// Rotates the entity at the position clockwise (or counter-clockwise).
    Rotate { position: MapPosition, reverse: bool },
    /// Queues hand crafting, crafting missing intermediates first.
    Craft { recipe: RecipeId, count: u32 },
    /// Cancels the queued craft at `index` and refunds its ingredients.
    CancelCraft { index: u32 },
    /// Moves items from the character's inventory into the entity at the position.
    TransferToEntity { position: MapPosition, item: ItemId, count: u32 },
    /// Takes everything from the entity's output (or whole inventory for containers).
    TakeFromEntity { position: MapPosition },
    /// Sets an assembling machine's recipe; current contents go back to the character.
    SetRecipe { position: MapPosition, recipe: Option<RecipeId> },
    /// Picks up items on the ground and on belts next to the character.
    PickupItems,
    /// Sandbox/testing: adds items to the character's inventory.
    CheatItems { item: ItemId, count: u32 },
    /// Sandbox/testing: places an entity without needing the item or reach.
    CheatPlaceEntity { entity: EntityProtoId, position: MapPosition, direction: Direction },
    /// Sandbox/testing: inserts items into the entity at the position without reach.
    CheatInsert { position: MapPosition, item: ItemId, count: u32 },
    /// Sandbox/testing: sets a machine's recipe without reach.
    CheatSetRecipe { position: MapPosition, recipe: RecipeId },
}

/// Inputs from one player for one tick.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerInput {
    pub player: u16,
    pub action: InputAction,
}

impl PlayerInput {
    pub fn new(player: u16, action: InputAction) -> Self {
        PlayerInput { player, action }
    }
}
