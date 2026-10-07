//! Player inputs: the only way anything outside the simulation can change it.
//!
//! In lockstep multiplayer, peers exchange these per tick and every peer applies the
//! same list in the same order, so they must be plain data with a total order.

use crate::map::{Direction, MapPosition};
use crate::proto::{EntityProtoId, ItemId, RecipeId};

/// Which inventory of an entity a GUI slot belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntityInventory {
    /// A container's contents.
    Main,
    Fuel,
    BurntResult,
    /// A crafting machine's ingredients (furnace source).
    Input,
    Output,
}

/// A slot shown in the GUI: the character's main inventory or the opened entity's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SlotRef {
    Character(u16),
    Opened(EntityInventory, u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MouseButton {
    Left,
    Right,
}

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
    /// Opens (or with `None` closes) the window of the entity at the position.
    OpenEntity(Option<MapPosition>),
    /// A click on an inventory slot, with Factorio's cursor semantics.
    ClickSlot { slot: SlotRef, button: MouseButton, shift: bool, ctrl: bool },
    /// Puts the cursor stack back into the inventory (Q).
    ClearCursor,
    /// Takes a stack of the item from the inventory into the cursor (quickbar keys, pipette).
    PickItem(ItemId),
    /// Assigns (or clears) a quickbar slot.
    SetQuickbar { index: u8, item: Option<ItemId> },
    /// Ctrl+click on an entity in the world: insert the cursor stack, or with an empty cursor
    /// take its output. `half` is Ctrl+right click.
    FastTransfer { position: MapPosition, half: bool },
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
