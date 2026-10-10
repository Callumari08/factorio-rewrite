//! Player inputs: the only way anything outside the simulation can change it.
//!
//! In lockstep multiplayer, peers exchange these per tick and every peer applies the
//! same list in the same order, so they must be plain data with a total order.

use crate::fixed::Fixed;
use crate::map::{Direction, MapPosition};
use crate::proto::{EntityProtoId, FluidId, ItemId, RecipeId, TechId};

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
    Build {
        item: ItemId,
        position: MapPosition,
        direction: Direction,
    },
    /// Rotates the entity at the position clockwise (or counter-clockwise).
    Rotate {
        position: MapPosition,
        reverse: bool,
    },
    /// Queues hand crafting, crafting missing intermediates first.
    Craft {
        recipe: RecipeId,
        count: u32,
    },
    /// Cancels the queued craft at `index` and refunds its ingredients.
    CancelCraft {
        index: u32,
    },
    /// Moves items from the character's inventory into the entity at the position.
    TransferToEntity {
        position: MapPosition,
        item: ItemId,
        count: u32,
    },
    /// Takes everything from the entity's output (or whole inventory for containers).
    TakeFromEntity {
        position: MapPosition,
    },
    /// Sets an assembling machine's recipe; current contents go back to the character.
    SetRecipe {
        position: MapPosition,
        recipe: Option<RecipeId>,
    },
    /// Picks up items on the ground and on belts next to the character.
    PickupItems,
    /// Opens (or with `None` closes) the window of the entity at the position.
    OpenEntity(Option<MapPosition>),
    /// A click on an inventory slot, with Factorio's cursor semantics.
    ClickSlot {
        slot: SlotRef,
        button: MouseButton,
        shift: bool,
        ctrl: bool,
    },
    /// Left-dragging the cursor stack across slots: the stack (as it was when the drag
    /// began) is split evenly over `slots`, all the slots dragged over so far. Sent again
    /// as each new slot is entered; `EndSpread` finishes the drag.
    SpreadCursor {
        slots: Vec<SlotRef>,
    },
    EndSpread,
    /// Sets the opened container's limit (the game's red X): slots from `bar` on are not
    /// filled by inserters or transfers. `None` removes it.
    SetContainerLimit(Option<u16>),
    /// Sets (or clears) a character inventory slot's filter (middle click).
    SetSlotFilter {
        slot: u16,
        item: Option<ItemId>,
    },
    /// Sets one of the opened inserter's filters.
    SetInserterFilter {
        index: u8,
        item: Option<ItemId>,
    },
    /// Turns the opened inserter's filters on or off, as a whitelist or blacklist.
    SetInserterFilterMode {
        use_filters: bool,
        blacklist: bool,
    },
    /// Sets (or clears) the opened inserter's stack size override.
    SetInserterStackOverride(Option<u32>),
    /// Puts a spawnable item (copper wire and the like) into the empty cursor, as the
    /// shortcut bar's spawn-item buttons do.
    SpawnItem(ItemId),
    /// With a wire in the cursor: wires the poles at `a` and `b` together, or removes the
    /// wire if they already are.
    WirePoles {
        a: MapPosition,
        b: MapPosition,
    },
    /// Removes all copper wires of the pole at the position (Shift+click with copper wire).
    ClearPoleWires(MapPosition),
    /// Puts the cursor stack back into the inventory (Q).
    ClearCursor,
    /// Takes a stack of the item from the inventory into the cursor (quickbar keys, pipette).
    PickItem(ItemId),
    /// Assigns (or clears) a quickbar slot.
    SetQuickbar {
        index: u8,
        item: Option<ItemId>,
    },
    /// Ctrl+click on an entity in the world: insert the cursor stack, or with an empty cursor
    /// take its output. `half` is Ctrl+right click.
    FastTransfer {
        position: MapPosition,
        half: bool,
    },
    /// Sandbox: a full stack of every item; what does not fit goes into chests placed
    /// next to the character.
    CheatAllItems,
    /// Factorio's `/cheat`: instant crafting without ingredients.
    SetCheatMode(bool),
    /// Adds a technology (and missing prerequisites) to the research queue, at the front
    /// when `front` is set.
    QueueResearch {
        tech: TechId,
        front: bool,
    },
    /// Removes a technology, and queued technologies needing it, from the research queue.
    DequeueResearch(TechId),
    /// Researches every technology (Factorio's `/cheat all`).
    CheatResearchAll,
    /// Sandbox/testing: moves the character (Factorio's `/c player.teleport`).
    CheatTeleport(MapPosition),
    /// Sandbox/testing: adds items to the character's inventory.
    CheatItems {
        item: ItemId,
        count: u32,
    },
    /// Sandbox/testing: places an entity without needing the item or reach.
    CheatPlaceEntity {
        entity: EntityProtoId,
        position: MapPosition,
        direction: Direction,
    },
    /// Sandbox/testing: inserts items into the entity at the position without reach.
    CheatInsert {
        position: MapPosition,
        item: ItemId,
        count: u32,
    },
    /// Sandbox/testing: puts fluid into the entity's first fluid box that its recipe (or
    /// it) takes in.
    CheatFluid {
        position: MapPosition,
        fluid: FluidId,
        amount: Fixed,
    },
    /// Sandbox/testing: sets a machine's recipe without reach.
    CheatSetRecipe {
        position: MapPosition,
        recipe: RecipeId,
    },
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
