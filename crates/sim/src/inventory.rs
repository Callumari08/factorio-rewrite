//! Slot-based item inventories.

use std::collections::BTreeMap;

use crate::proto::{ItemId, PrototypeDb};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemStack {
    pub item: ItemId,
    pub count: u32,
}

impl ItemStack {
    pub const fn new(item: ItemId, count: u32) -> Self {
        ItemStack { item, count }
    }
}

/// A fixed number of slots, each holding at most one stack of one item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Inventory {
    slots: Vec<Option<ItemStack>>,
}

impl Inventory {
    pub fn new(size: u32) -> Self {
        Inventory { slots: vec![None; size as usize] }
    }

    /// Adds empty slots up to `size` (never removes slots).
    pub fn grow_to(&mut self, size: u32) {
        if self.slots.len() < size as usize {
            self.slots.resize(size as usize, None);
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    pub fn slots(&self) -> &[Option<ItemStack>] {
        &self.slots
    }

    pub fn slot(&self, i: usize) -> Option<ItemStack> {
        self.slots.get(i).copied().flatten()
    }

    pub fn set_slot(&mut self, i: usize, stack: Option<ItemStack>) {
        self.slots[i] = stack.filter(|s| s.count > 0);
    }

    pub fn count(&self, item: ItemId) -> u32 {
        self.slots.iter().flatten().filter(|s| s.item == item).map(|s| s.count).sum()
    }

    /// How many of `item` would fit.
    pub fn space_for(&self, db: &PrototypeDb, item: ItemId) -> u32 {
        let stack = db.item(item).stack_size;
        self.slots
            .iter()
            .map(|s| match s {
                None => stack,
                Some(s) if s.item == item => stack.saturating_sub(s.count),
                Some(_) => 0,
            })
            .sum()
    }

    /// Inserts up to `count`, topping up existing stacks before using empty slots.
    /// Returns how many were inserted.
    pub fn insert(&mut self, db: &PrototypeDb, item: ItemId, count: u32) -> u32 {
        let stack_size = db.item(item).stack_size;
        let mut left = count;
        for s in self.slots.iter_mut().flatten() {
            if left == 0 {
                break;
            }
            if s.item == item && s.count < stack_size {
                let n = left.min(stack_size - s.count);
                s.count += n;
                left -= n;
            }
        }
        for s in self.slots.iter_mut() {
            if left == 0 {
                break;
            }
            if s.is_none() {
                let n = left.min(stack_size);
                *s = Some(ItemStack::new(item, n));
                left -= n;
            }
        }
        count - left
    }

    /// Removes up to `count`, taking from the last slots first. Returns how many were removed.
    pub fn remove(&mut self, item: ItemId, count: u32) -> u32 {
        let mut left = count;
        for s in self.slots.iter_mut().rev() {
            if left == 0 {
                break;
            }
            if let Some(stack) = s
                && stack.item == item
            {
                let n = left.min(stack.count);
                stack.count -= n;
                left -= n;
                if stack.count == 0 {
                    *s = None;
                }
            }
        }
        count - left
    }

    /// The item in the first non-empty slot.
    pub fn first_item(&self) -> Option<ItemId> {
        self.slots.iter().flatten().next().map(|s| s.item)
    }

    /// Item totals in id order.
    pub fn contents(&self) -> BTreeMap<ItemId, u32> {
        let mut out = BTreeMap::new();
        for s in self.slots.iter().flatten() {
            *out.entry(s.item).or_insert(0) += s.count;
        }
        out
    }

    /// Removes and returns everything.
    pub fn take_all(&mut self) -> Vec<ItemStack> {
        self.slots.iter_mut().filter_map(Option::take).collect()
    }

    /// Merges stacks and orders them by item id, like Factorio's automatic sorting of the
    /// character's main inventory.
    pub fn sort_and_merge(&mut self, db: &PrototypeDb) {
        let mut contents: Vec<(ItemId, u32)> = self.contents().into_iter().collect();
        contents.sort_by_key(|(i, _)| (db.item(*i).sort_index, *i));
        let size = self.slots.len() as u32;
        *self = Inventory::new(size);
        for (item, count) in contents {
            self.insert(db, item, count);
        }
    }
}
