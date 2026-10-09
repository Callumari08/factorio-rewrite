//! The cursor stack and inventory slot clicks, following Factorio's GUI rules.

use crate::energy::Burner;
use crate::input::{EntityInventory, MouseButton, SlotRef};
use crate::inventory::{Inventory, ItemStack};
use crate::machines::CrafterState;
use crate::proto::{EntityData, ItemId, ItemOrFluid, PrototypeDb};
use crate::world::{EntityId, EntityState, InsertSource, Simulation};

/// The entity inventory behind a GUI slot, if the entity has it.
pub(crate) fn entity_inventory(state: &mut EntityState, which: EntityInventory) -> Option<&mut Inventory> {
    let energy = match state {
        EntityState::Drill(d) => Some(&mut d.energy),
        EntityState::Inserter(i) => Some(&mut i.energy),
        EntityState::Fluid(f) => Some(&mut f.energy),
        EntityState::Crafter(c) => match which {
            EntityInventory::Input => return Some(&mut c.input),
            EntityInventory::Output => return Some(&mut c.output),
            _ => Some(&mut c.energy),
        },
        EntityState::Container(inv) => return (which == EntityInventory::Main).then_some(inv),
        EntityState::Lab(l) => match which {
            EntityInventory::Input => return Some(&mut l.input),
            _ => None,
        },
        _ => None,
    }?;
    let b = energy.burner_mut()?;
    match which {
        EntityInventory::Fuel => Some(&mut b.fuel),
        EntityInventory::BurntResult => Some(&mut b.burnt),
        _ => None,
    }
}

/// Whether the player may put `item` into this slot by hand.
fn slot_accepts(
    db: &PrototypeDb,
    sim: &Simulation,
    entity: EntityId,
    which: EntityInventory,
    slot: usize,
    item: ItemId,
) -> bool {
    let Some(e) = sim.entity(entity) else { return false };
    let proto = db.entity(e.proto);
    match which {
        EntityInventory::Main => true,
        EntityInventory::Output | EntityInventory::BurntResult => false,
        EntityInventory::Fuel => proto.energy_source().is_some_and(|s| Burner::accepts(db, s, item)),
        EntityInventory::Input => match &e.state {
            EntityState::Crafter(c) if c.furnace => {
                CrafterState::new(proto).ingredient_room(db, &sim.research, proto, item, InsertSource::Player) > 0
                    && c.input.first_item().is_none_or(|i| i == item)
            }
            EntityState::Crafter(c) => c
                .recipe
                .is_some_and(|r| db.recipe(r).ingredients.get(slot).is_some_and(|i| i.what == ItemOrFluid::Item(item))),
            EntityState::Lab(_) => {
                matches!(&proto.data, EntityData::Lab { inputs, .. } if inputs.get(slot) == Some(&item))
            }
            _ => false,
        },
    }
}

fn character_inventory(sim: &mut Simulation, player: u16) -> Option<&mut Inventory> {
    Some(&mut sim.players.get_mut(&player)?.character.as_mut()?.inventory)
}

fn cursor(sim: &mut Simulation, player: u16) -> Option<&mut Option<ItemStack>> {
    Some(&mut sim.players.get_mut(&player)?.character.as_mut()?.cursor)
}

pub(crate) fn click_slot(
    sim: &mut Simulation,
    player: u16,
    slot: SlotRef,
    button: MouseButton,
    shift: bool,
    ctrl: bool,
) {
    let db = sim.db.clone();
    let opened = sim.players.get(&player).and_then(|p| p.opened).filter(|id| sim.entity(*id).is_some());
    if shift || ctrl {
        transfer(sim, player, opened, slot, ctrl);
        return;
    }
    // Read the slot and decide what is allowed before mutating anything.
    let (contents, accepts_cursor) = {
        let cur = sim.players.get(&player).and_then(|p| p.character.as_ref()).and_then(|c| c.cursor);
        match slot {
            SlotRef::Character(i) => {
                let inv = &sim.players[&player].character.as_ref().unwrap().inventory;
                if i as usize >= inv.len() {
                    return;
                }
                (inv.slot(i as usize), cur.is_some_and(|c| inv.allows(i as usize, c.item)))
            }
            SlotRef::Opened(which, i) => {
                let Some(id) = opened else { return };
                let mut probe = sim.entity(id).unwrap().state.clone();
                let Some(inv) = entity_inventory(&mut probe, which) else { return };
                if i as usize >= inv.len() {
                    return;
                }
                let ok = cur.is_some_and(|c| slot_accepts(&db, sim, id, which, i as usize, c.item));
                (inv.slot(i as usize), ok)
            }
        }
    };
    let cur = *cursor(sim, player).unwrap();
    let stack_size = |i: ItemId| db.item(i).stack_size;
    let (new_slot, new_cursor) = match (button, cur, contents) {
        (MouseButton::Left, None, Some(s)) => (None, Some(s)),
        (MouseButton::Left, Some(c), None) if accepts_cursor => {
            let n = c.count.min(stack_size(c.item));
            (Some(ItemStack::new(c.item, n)), (c.count > n).then(|| ItemStack::new(c.item, c.count - n)))
        }
        (MouseButton::Left, Some(c), Some(s)) if c.item == s.item && accepts_cursor => {
            let n = c.count.min(stack_size(s.item).saturating_sub(s.count));
            (Some(ItemStack::new(s.item, s.count + n)), (c.count > n).then(|| ItemStack::new(c.item, c.count - n)))
        }
        (MouseButton::Left, Some(c), Some(s)) if accepts_cursor && c.count <= stack_size(c.item) => (Some(c), Some(s)),
        (MouseButton::Right, None, Some(s)) => {
            let take = s.count.div_ceil(2);
            ((s.count > take).then(|| ItemStack::new(s.item, s.count - take)), Some(ItemStack::new(s.item, take)))
        }
        (MouseButton::Right, Some(c), slot_contents) if accepts_cursor => match slot_contents {
            None => (Some(ItemStack::new(c.item, 1)), (c.count > 1).then(|| ItemStack::new(c.item, c.count - 1))),
            Some(s) if s.item == c.item && s.count < stack_size(s.item) => {
                (Some(ItemStack::new(s.item, s.count + 1)), (c.count > 1).then(|| ItemStack::new(c.item, c.count - 1)))
            }
            other => (other, Some(c)),
        },
        _ => return,
    };
    match slot {
        SlotRef::Character(i) => {
            let inv = character_inventory(sim, player).unwrap();
            inv.set_slot(i as usize, new_slot);
            // Picking up a whole stack leaves the hand in its slot, kept free for it.
            if cur.is_none() && new_slot.is_none() && new_cursor.is_some() {
                inv.set_reserved(Some(i as usize));
            }
        }
        SlotRef::Opened(which, i) => {
            let id = opened.unwrap();
            let e = sim.entities.get_mut(&id).unwrap();
            entity_inventory(&mut e.state, which).unwrap().set_slot(i as usize, new_slot);
        }
    }
    *cursor(sim, player).unwrap() = new_cursor;
}

/// Shift-click (one stack) and ctrl-click (all of that item) between the character and
/// the opened entity.
fn transfer(sim: &mut Simulation, player: u16, opened: Option<EntityId>, slot: SlotRef, all: bool) {
    let db = sim.db.clone();
    let Some(id) = opened else { return };
    match slot {
        SlotRef::Character(i) => {
            let Some(s) = sim.players[&player].character.as_ref().unwrap().inventory.slot(i as usize) else { return };
            let have = character_inventory(sim, player).unwrap().count(s.item);
            let count = if all { have } else { s.count };
            let n = sim.insert_into_entity(id, s.item, count, InsertSource::Player);
            character_inventory(sim, player).unwrap().remove(s.item, n);
        }
        SlotRef::Opened(which, i) => {
            let e = sim.entities.get_mut(&id).unwrap();
            let Some(inv) = entity_inventory(&mut e.state, which) else { return };
            let Some(s) = inv.slot(i as usize) else { return };
            let count = if all { inv.count(s.item) } else { s.count };
            let room = sim.players[&player].character.as_ref().unwrap().inventory.space_for(&db, s.item);
            let n = count.min(room);
            let e = sim.entities.get_mut(&id).unwrap();
            entity_inventory(&mut e.state, which).unwrap().remove(s.item, n);
            character_inventory(sim, player).unwrap().insert(&db, s.item, n);
        }
    }
}

/// Returns the cursor stack to the inventory (spilling what does not fit).
pub(crate) fn clear_cursor(sim: &mut Simulation, player: u16) {
    let db = sim.db.clone();
    let Some(c) = sim.players.get_mut(&player).and_then(|p| p.character.as_mut()) else { return };
    let Some(stack) = c.cursor.take() else { return };
    // Back to the hand's slot first, as in the game.
    let mut n = 0;
    if let Some(r) = c.inventory.reserved().filter(|r| c.inventory.slot(*r).is_none()) {
        n = stack.count.min(db.item(stack.item).stack_size);
        c.inventory.set_slot(r, Some(ItemStack::new(stack.item, n)));
    }
    c.inventory.set_reserved(None);
    let n = n + c.inventory.insert(&db, stack.item, stack.count - n);
    let at = c.position();
    if n < stack.count {
        sim.spill(at, ItemStack::new(stack.item, stack.count - n));
    }
}

/// Puts one stack of `item` from the inventory into the cursor.
pub(crate) fn pick_item(sim: &mut Simulation, player: u16, item: ItemId) {
    let db = sim.db.clone();
    if sim
        .players
        .get(&player)
        .and_then(|p| p.character.as_ref())
        .and_then(|c| c.cursor)
        .is_some_and(|c| c.item == item)
    {
        return;
    }
    clear_cursor(sim, player);
    let Some(c) = sim.players.get_mut(&player).and_then(|p| p.character.as_mut()) else { return };
    let before: Vec<bool> = c.inventory.slots().iter().map(Option::is_some).collect();
    let n = c.inventory.remove(item, db.item(item).stack_size);
    if n > 0 {
        c.cursor = Some(ItemStack::new(item, n));
        // The hand goes where the stack was taken from.
        let emptied = before.iter().enumerate().find(|(i, had)| **had && c.inventory.slot(*i).is_none());
        c.inventory.set_reserved(emptied.map(|(i, _)| i));
    }
}

/// After building from the cursor: take one item and refill from the inventory when empty.
pub(crate) fn consume_cursor_item(sim: &mut Simulation, player: u16) {
    let db = sim.db.clone();
    let Some(c) = sim.players.get_mut(&player).and_then(|p| p.character.as_mut()) else { return };
    let Some(stack) = c.cursor.as_mut() else { return };
    stack.count -= 1;
    if stack.count == 0 {
        let item = stack.item;
        let n = c.inventory.remove(item, db.item(item).stack_size);
        c.cursor = (n > 0).then(|| ItemStack::new(item, n));
    }
}

/// Ctrl+click on an entity in the world.
pub(crate) fn fast_transfer(sim: &mut Simulation, player: u16, id: EntityId, half: bool) {
    let db = sim.db.clone();
    let cur = sim.players.get(&player).and_then(|p| p.character.as_ref()).and_then(|c| c.cursor);
    match cur {
        Some(stack) => {
            let count = if half { stack.count.div_ceil(2) } else { stack.count };
            let n = sim.insert_into_entity(id, stack.item, count, InsertSource::Player);
            let c = cursor(sim, player).unwrap();
            *c = (stack.count > n).then(|| ItemStack::new(stack.item, stack.count - n));
        }
        None => {
            // Take the output (or a container's contents).
            let mut taken: Vec<ItemStack> = Vec::new();
            if let Some(e) = sim.entities.get_mut(&id) {
                let inv = match &mut e.state {
                    EntityState::Container(inv) => Some(inv),
                    EntityState::Crafter(c) => Some(&mut c.output),
                    _ => None,
                };
                if let Some(inv) = inv {
                    for (item, count) in inv.contents() {
                        let n = if half { count.div_ceil(2) } else { count };
                        inv.remove(item, n);
                        taken.push(ItemStack::new(item, n));
                    }
                }
            }
            for s in taken {
                let c = character_inventory(sim, player).unwrap();
                let n = c.insert(&db, s.item, s.count);
                if n < s.count {
                    sim.insert_into_entity(id, s.item, s.count - n, InsertSource::Player);
                }
            }
        }
    }
}

/// Whether this entity has a GUI window worth opening.
pub fn has_window(db: &PrototypeDb, sim: &Simulation, id: EntityId) -> bool {
    sim.entity(id).is_some_and(|e| {
        !matches!(
            db.entity(e.proto).data,
            EntityData::TransportBelt { .. } | EntityData::Pipe { .. } | EntityData::Other
        )
    })
}

/// Drops the hand once the cursor is empty or its slot has been filled.
pub(crate) fn sync_hand(sim: &mut Simulation, player: u16) {
    let Some(c) = sim.players.get_mut(&player).and_then(|p| p.character.as_mut()) else { return };
    if let Some(r) = c.inventory.reserved()
        && (c.cursor.is_none() || c.inventory.slot(r).is_some())
    {
        c.inventory.set_reserved(None);
    }
}

/// A drag-spread in progress: what the cursor held when it began, and what each slot
/// held before items were spread into it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Spread {
    pub stack: ItemStack,
    pub before: Vec<(SlotRef, Option<ItemStack>)>,
}

/// Reads a slot (character or opened entity): `None` when there is no such slot.
pub fn read_slot(sim: &Simulation, player: u16, slot: SlotRef) -> Option<Option<ItemStack>> {
    match slot {
        SlotRef::Character(i) => {
            let inv = &sim.players.get(&player)?.character.as_ref()?.inventory;
            ((i as usize) < inv.len()).then(|| inv.slot(i as usize))
        }
        SlotRef::Opened(which, i) => {
            let id = sim.players.get(&player)?.opened?;
            let mut probe = sim.entity(id)?.state.clone();
            let inv = entity_inventory(&mut probe, which)?;
            ((i as usize) < inv.len()).then(|| inv.slot(i as usize))
        }
    }
}

fn write_slot(sim: &mut Simulation, player: u16, slot: SlotRef, stack: Option<ItemStack>) {
    match slot {
        SlotRef::Character(i) => {
            if let Some(inv) = character_inventory(sim, player) {
                inv.set_slot(i as usize, stack);
            }
        }
        SlotRef::Opened(which, i) => {
            let Some(id) = sim.players.get(&player).and_then(|p| p.opened) else { return };
            if let Some(e) = sim.entities.get_mut(&id)
                && let Some(inv) = entity_inventory(&mut e.state, which)
            {
                inv.set_slot(i as usize, stack);
            }
        }
    }
}

/// Spreads the stack the cursor held when the drag began evenly over `slots` (those
/// that are empty or hold the same item, and accept it), as the game's left drag. What
/// does not fit stays in the cursor.
pub(crate) fn spread(sim: &mut Simulation, player: u16, slots: Vec<SlotRef>) {
    let db = sim.db.clone();
    let Some(c) = sim.players.get(&player).and_then(|p| p.character.as_ref()) else { return };
    // Undo the previous step of this drag.
    let state = match c.spread.clone() {
        Some(s) => {
            for (slot, before) in s.before.iter().rev() {
                write_slot(sim, player, *slot, *before);
            }
            s
        }
        None => match c.cursor {
            Some(stack) => Spread { stack, before: Vec::new() },
            None => return,
        },
    };
    let stack = state.stack;
    let opened = sim.players.get(&player).and_then(|p| p.opened);
    let stack_size = db.item(stack.item).stack_size;
    // The slots that can take part.
    let mut targets: Vec<(SlotRef, Option<ItemStack>)> = Vec::new();
    for slot in slots {
        if targets.iter().any(|(s, _)| *s == slot) {
            continue;
        }
        let Some(contents) = read_slot(sim, player, slot) else { continue };
        let accepts = match slot {
            SlotRef::Character(i) => {
                character_inventory(sim, player).is_some_and(|inv| inv.allows(i as usize, stack.item))
            }
            SlotRef::Opened(which, i) => {
                opened.is_some_and(|id| slot_accepts(&db, sim, id, which, i as usize, stack.item))
            }
        };
        if accepts && contents.is_none_or(|s| s.item == stack.item && s.count < stack_size) {
            targets.push((slot, contents));
        }
    }
    let mut left = stack.count;
    let n = targets.len() as u32;
    for (k, (slot, contents)) in targets.iter().enumerate() {
        if n == 0 {
            break;
        }
        // Even shares, the remainder to the first slots.
        let share = stack.count / n + u32::from((k as u32) < stack.count % n);
        let have = contents.map_or(0, |s| s.count);
        let put = share.min(stack_size - have).min(left);
        if put > 0 {
            write_slot(sim, player, *slot, Some(ItemStack::new(stack.item, have + put)));
            left -= put;
        }
    }
    let c = sim.players.get_mut(&player).unwrap().character.as_mut().unwrap();
    c.cursor = (left > 0).then(|| ItemStack::new(stack.item, left));
    c.spread = Some(Spread { stack, before: targets });
}
