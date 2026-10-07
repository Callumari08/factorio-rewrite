//! Players and their characters: walking, hand mining, hand crafting and building.

use std::collections::BTreeMap;

use crate::fixed::Fixed;
use crate::input::InputAction;
use crate::inventory::{Inventory, ItemStack};
use crate::map::{Area, Direction, MapPosition, SUBTILES_PER_TILE};
use crate::proto::{EntityData, EntityProtoId, ItemId, ItemOrFluid, PrototypeDb, RecipeId};
use crate::world::{EntityId, EntityState, InsertSource, Simulation};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Player {
    pub character: Option<Character>,
    /// The entity whose window this player has open (Factorio's `player.opened`).
    pub opened: Option<EntityId>,
    /// Quickbar shortcuts: items, not storage (2 rows of 10).
    pub quickbar: [Option<ItemId>; QUICKBAR_SLOTS],
}

pub const QUICKBAR_SLOTS: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MiningTarget {
    Entity(EntityId),
    Resource(crate::map::TilePosition),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Mining {
    pub position: MapPosition,
    pub target: MiningTarget,
    /// Accumulated mining speed; the target is mined when this reaches `mining_time * 60`.
    pub progress: Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CraftJob {
    pub recipe: RecipeId,
    pub count: u32,
    /// Items of this job's product already promised to a later job in the queue; they are
    /// consumed directly instead of going into the inventory.
    pub reserved: u32,
    /// Ingredients taken from the inventory for this job, refunded on cancel.
    pub consumed: Vec<ItemStack>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Character {
    pub proto: EntityProtoId,
    /// Position in tiles with fixed-point precision.
    pub x: Fixed,
    pub y: Fixed,
    pub walking: Option<Direction>,
    pub inventory: Inventory,
    pub mining: Option<Mining>,
    pub queue: Vec<CraftJob>,
    /// Ticks of crafting done on the first job's current craft (at crafting speed 1).
    pub craft_progress: Fixed,
    /// The stack held on the mouse cursor.
    pub cursor: Option<ItemStack>,
}

struct CharacterStats {
    running_speed: Fixed,
    mining_speed: Fixed,
    build_distance: Fixed,
    reach_distance: Fixed,
    reach_resource_distance: Fixed,
    item_pickup_distance: Fixed,
    mining_categories: Vec<String>,
    crafting_categories: Vec<String>,
}

fn stats(db: &PrototypeDb, proto: EntityProtoId) -> CharacterStats {
    match &db.entity(proto).data {
        EntityData::Character {
            running_speed,
            mining_speed,
            build_distance,
            reach_distance,
            reach_resource_distance,
            item_pickup_distance,
            mining_categories,
            crafting_categories,
            ..
        } => CharacterStats {
            running_speed: *running_speed,
            mining_speed: *mining_speed,
            build_distance: *build_distance,
            reach_distance: *reach_distance,
            reach_resource_distance: *reach_resource_distance,
            item_pickup_distance: *item_pickup_distance,
            mining_categories: mining_categories.clone(),
            crafting_categories: crafting_categories.clone(),
        },
        _ => panic!("not a character prototype"),
    }
}

impl Character {
    pub fn new(db: &PrototypeDb, proto: EntityProtoId, at: MapPosition) -> Self {
        let size = match &db.entity(proto).data {
            EntityData::Character { inventory_size, .. } => *inventory_size,
            _ => 0,
        };
        Character {
            proto,
            x: Fixed::from_ratio(at.x as i64, SUBTILES_PER_TILE as i64),
            y: Fixed::from_ratio(at.y as i64, SUBTILES_PER_TILE as i64),
            walking: None,
            inventory: Inventory::new(size),
            mining: None,
            queue: Vec::new(),
            craft_progress: Fixed::ZERO,
            cursor: None,
        }
    }

    pub fn position(&self) -> MapPosition {
        Simulation::fixed_to_map(self.x, self.y)
    }

    pub fn collision_area(&self, db: &PrototypeDb) -> Area {
        db.entity(self.proto).collision_box.at(self.position())
    }
}

/// Distance in tiles between the character and the closest point of an area.
fn distance_to_area(from: MapPosition, area: &Area) -> Fixed {
    let dx = (area.left_top.x - from.x).max(0).max(from.x - area.right_bottom.x);
    let dy = (area.left_top.y - from.y).max(0).max(from.y - area.right_bottom.y);
    let d2 = dx as i64 * dx as i64 + dy as i64 * dy as i64;
    Fixed::from_raw((d2 as u64).isqrt() as i64 * (1 << Fixed::FRAC_BITS) / SUBTILES_PER_TILE as i64)
}

pub(crate) fn character_fits(sim: &Simulation, proto: EntityProtoId, at: MapPosition) -> bool {
    let db = &sim.db;
    let cp = db.entity(proto);
    let area = cp.collision_box.at(at);
    for t in area.tiles() {
        match sim.surface.tile(t) {
            None => return false,
            Some(tile) => {
                if db.tile(tile).collision_mask.collides(&cp.collision_mask) {
                    return false;
                }
            }
        }
    }
    for id in sim.entities_in(area) {
        let e = &sim.entities[&id];
        let ep = db.entity(e.proto);
        if ep.collision_mask.collides(&cp.collision_mask) && ep.rotated_box(e.direction).at(e.position).overlaps(&area)
        {
            return false;
        }
    }
    true
}

pub(crate) fn apply_input(sim: &mut Simulation, player: u16, action: &InputAction) {
    let db = sim.db.clone();
    let Some(c) = sim.players.get(&player).and_then(|p| p.character.as_ref()) else { return };
    let st = stats(&db, c.proto);
    let me = c.position();
    match *action {
        InputAction::SetWalking(d) => character_mut(sim, player).walking = d,
        InputAction::SetMining(None) => character_mut(sim, player).mining = None,
        InputAction::SetMining(Some(p)) => {
            let target = if let Some(id) = sim.entity_at(p) {
                Some(MiningTarget::Entity(id))
            } else if sim.surface.resource(p.tile()).is_some() {
                Some(MiningTarget::Resource(p.tile()))
            } else {
                None
            };
            let c = character_mut(sim, player);
            let same = c.mining.as_ref().is_some_and(|m| Some(&m.target) == target.as_ref());
            if !same {
                c.mining = target.map(|target| Mining { position: p, target, progress: Fixed::ZERO });
            }
        }
        InputAction::Build { item, position, direction } => {
            let Some(entity) = db.item(item).place_result else { return };
            // Build from the cursor stack when it holds this item, else from the inventory.
            let from_cursor = c.cursor.is_some_and(|s| s.item == item);
            if !from_cursor && c.inventory.count(item) == 0 {
                return;
            }
            let proto = db.entity(entity);
            let snapped = proto.snap_position(position, direction);
            if distance_to_area(me, &Simulation::footprint(proto, snapped, direction)) > st.build_distance {
                return;
            }
            if sim.place_entity(entity, position, direction).is_ok() {
                if from_cursor {
                    crate::cursor::consume_cursor_item(sim, player);
                } else {
                    character_mut(sim, player).inventory.remove(item, 1);
                }
            }
        }
        InputAction::Rotate { position, reverse } => {
            if let Some(id) = sim.entity_at(position) {
                sim.rotate_entity(id, reverse);
            }
        }
        InputAction::Craft { recipe, count } => queue_craft(sim, player, &st.crafting_categories, recipe, count),
        InputAction::CancelCraft { index } => {
            let c = character_mut(sim, player);
            let i = index as usize;
            if i < c.queue.len() {
                let job = c.queue.remove(i);
                if i == 0 {
                    c.craft_progress = Fixed::ZERO;
                }
                for s in job.consumed {
                    give(sim, player, s.item, s.count);
                }
            }
        }
        InputAction::TransferToEntity { position, item, count } => {
            let Some(id) = sim.entity_at(position) else { return };
            if !in_reach(sim, me, id, st.reach_distance) {
                return;
            }
            let have = character_mut(sim, player).inventory.count(item);
            let n = sim.insert_into_entity(id, item, count.min(have), InsertSource::Player);
            character_mut(sim, player).inventory.remove(item, n);
        }
        InputAction::TakeFromEntity { position } => {
            let Some(id) = sim.entity_at(position) else { return };
            if !in_reach(sim, me, id, st.reach_distance) {
                return;
            }
            let mut taken = Vec::new();
            if let Some(e) = sim.entities.get_mut(&id) {
                match &mut e.state {
                    EntityState::Container(inv) => taken = inv.take_all(),
                    EntityState::Crafter(c) => taken = c.output.take_all(),
                    _ => {}
                }
            }
            for s in taken {
                let n = character_mut(sim, player).inventory.insert(&db, s.item, s.count);
                if n < s.count {
                    // Put back what does not fit.
                    sim.insert_into_entity(id, s.item, s.count - n, InsertSource::Player);
                }
            }
        }
        InputAction::SetRecipe { position, recipe } => {
            let Some(id) = sim.entity_at(position) else { return };
            if !in_reach(sim, me, id, st.reach_distance) {
                return;
            }
            let mut returned = Vec::new();
            if let Some(e) = sim.entities.get_mut(&id)
                && let EntityState::Crafter(c) = &mut e.state
            {
                returned = c.set_recipe(&db, db.entity(e.proto), recipe);
            }
            for s in returned {
                give(sim, player, s.item, s.count);
            }
        }
        InputAction::PickupItems => pickup_items(sim, player, st.item_pickup_distance),
        InputAction::OpenEntity(p) => {
            let id = p.and_then(|p| sim.entity_at(p)).filter(|id| in_reach(sim, me, *id, st.reach_distance));
            sim.players.get_mut(&player).unwrap().opened = id;
        }
        InputAction::ClickSlot { slot, button, shift, ctrl } => {
            crate::cursor::click_slot(sim, player, slot, button, shift, ctrl);
        }
        InputAction::ClearCursor => crate::cursor::clear_cursor(sim, player),
        InputAction::PickItem(item) => crate::cursor::pick_item(sim, player, item),
        InputAction::SetQuickbar { index, item } => {
            if let Some(slot) = sim.players.get_mut(&player).unwrap().quickbar.get_mut(index as usize) {
                *slot = item;
            }
        }
        InputAction::FastTransfer { position, half } => {
            if let Some(id) = sim.entity_at(position)
                && in_reach(sim, me, id, st.reach_distance)
            {
                crate::cursor::fast_transfer(sim, player, id, half);
            }
        }
        InputAction::CheatItems { item, count } => give(sim, player, item, count),
        InputAction::JoinGame
        | InputAction::CheatPlaceEntity { .. }
        | InputAction::CheatInsert { .. }
        | InputAction::CheatSetRecipe { .. } => {}
    }
}

fn character_mut(sim: &mut Simulation, player: u16) -> &mut Character {
    sim.players.get_mut(&player).unwrap().character.as_mut().unwrap()
}

fn in_reach(sim: &Simulation, me: MapPosition, id: EntityId, reach: Fixed) -> bool {
    let e = &sim.entities[&id];
    let p = sim.db.entity(e.proto);
    distance_to_area(me, &Simulation::footprint(p, e.position, e.direction)) <= reach
}

/// Gives items to the character, spilling what does not fit on the ground.
fn give(sim: &mut Simulation, player: u16, item: ItemId, count: u32) {
    let db = sim.db.clone();
    let c = character_mut(sim, player);
    let n = c.inventory.insert(&db, item, count);
    let at = c.position();
    let stack = db.item(item).stack_size;
    let mut left = count - n;
    while left > 0 {
        let k = left.min(stack);
        sim.spill(at, ItemStack::new(item, k));
        left -= k;
    }
}

fn pickup_items(sim: &mut Simulation, player: u16, distance: Fixed) {
    let db = sim.db.clone();
    let c = character_mut(sim, player);
    let me = c.position();
    let r = (distance * Fixed::from_int(SUBTILES_PER_TILE as i64)).floor_int() as i32 + SUBTILES_PER_TILE / 2;
    let near: Vec<MapPosition> =
        sim.ground_items.keys().filter(|p| (p.x - me.x).abs() <= r && (p.y - me.y).abs() <= r).copied().collect();
    for p in near {
        let stack = sim.ground_items[&p];
        let n = character_mut(sim, player).inventory.insert(&db, stack.item, stack.count);
        if n == stack.count {
            sim.ground_items.remove(&p);
        } else {
            sim.ground_items.get_mut(&p).unwrap().count -= n;
        }
    }
    // Belts within reach: take everything on the belt tiles next to the character.
    let area = Area { left_top: me.offset(-r, -r), right_bottom: me.offset(r, r) };
    for t in area.tiles() {
        let Some(id) = sim.belts.at_tile(t) else { continue };
        loop {
            let c = character_mut(sim, player);
            let inv = c.inventory.clone();
            let Some(b) = sim.belts.get_mut(id) else { break };
            let lanes: Vec<usize> = (0..b.lanes.len()).collect();
            let Some(item) = b.take(&lanes, |i| inv.space_for(&db, i) > 0) else { break };
            character_mut(sim, player).inventory.insert(&db, item, 1);
        }
    }
}

// ----- crafting -----

fn hand_craftable(db: &PrototypeDb, categories: &[String], r: RecipeId) -> bool {
    let rec = db.recipe(r);
    categories.contains(&rec.category)
        && rec.ingredients.iter().all(|i| matches!(i.what, ItemOrFluid::Item(_)))
        && rec.results.iter().all(|p| matches!(p.what, ItemOrFluid::Item(_)))
}

fn product_count(db: &PrototypeDb, r: RecipeId, item: ItemId) -> u32 {
    db.recipe(r).results.iter().filter(|p| p.what == ItemOrFluid::Item(item)).filter_map(|p| p.fixed_count()).sum()
}

/// Plans `count` crafts of `recipe` against a virtual inventory, adding intermediate jobs
/// for missing ingredients first. Returns false if the materials are not available.
fn plan(
    db: &PrototypeDb,
    categories: &[String],
    recipe: RecipeId,
    count: u32,
    inv: &mut BTreeMap<ItemId, u32>,
    jobs: &mut Vec<CraftJob>,
    depth: u32,
) -> bool {
    if depth > 16 || !hand_craftable(db, categories, recipe) {
        return false;
    }
    let mut consumed = Vec::new();
    for ing in &db.recipe(recipe).ingredients {
        let ItemOrFluid::Item(item) = ing.what else { return false };
        let need = ing.amount.floor_int() as u32 * count;
        let have = inv.get(&item).copied().unwrap_or(0);
        let from_inv = need.min(have);
        if from_inv > 0 {
            *inv.get_mut(&item).unwrap() -= from_inv;
            consumed.push(ItemStack::new(item, from_inv));
        }
        let missing = need - from_inv;
        if missing > 0 {
            let sub = db.recipes_producing(item).into_iter().find(|r| {
                db.recipe(*r).allow_as_intermediate
                    && hand_craftable(db, categories, *r)
                    && product_count(db, *r, item) > 0
            });
            let Some(sub) = sub else { return false };
            let per = product_count(db, sub, item);
            let crafts = missing.div_ceil(per);
            if !plan(db, categories, sub, crafts, inv, jobs, depth + 1) {
                return false;
            }
            jobs.last_mut().unwrap().reserved += missing;
        }
    }
    jobs.push(CraftJob { recipe, count, reserved: 0, consumed });
    true
}

/// How many crafts of `recipe` the inventory allows, including intermediates.
pub fn max_craftable(db: &PrototypeDb, categories: &[String], inventory: &Inventory, recipe: RecipeId) -> u32 {
    let feasible = |n: u32| {
        let mut inv = inventory.contents();
        plan(db, categories, recipe, n, &mut inv, &mut Vec::new(), 0)
    };
    if !feasible(1) {
        return 0;
    }
    let (mut lo, mut hi) = (1u32, 2u32);
    while hi < 100_000 && feasible(hi) {
        lo = hi;
        hi *= 2;
    }
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if feasible(mid) { lo = mid } else { hi = mid }
    }
    lo
}

fn queue_craft(sim: &mut Simulation, player: u16, categories: &[String], recipe: RecipeId, count: u32) {
    let db = sim.db.clone();
    let c = character_mut(sim, player);
    // `u32::MAX` means "as many as possible" (shift-click).
    let count = if count == u32::MAX { max_craftable(&db, categories, &c.inventory, recipe) } else { count };
    let mut inv = c.inventory.contents();
    let mut jobs = Vec::new();
    if count == 0 || !plan(&db, categories, recipe, count, &mut inv, &mut jobs, 0) {
        return;
    }
    for job in &jobs {
        for s in &job.consumed {
            c.inventory.remove(s.item, s.count);
        }
    }
    c.queue.extend(jobs);
}

// ----- per-tick update -----

pub(crate) fn update(sim: &mut Simulation, player: u16) {
    if sim.players.get(&player).and_then(|p| p.character.as_ref()).is_none() {
        return;
    }
    walk(sim, player);
    mine(sim, player);
    craft(sim, player);
    // Close the open window when its entity is gone or out of reach.
    let me = sim.players[&player].character.as_ref().unwrap();
    let reach = stats(&sim.db, me.proto).reach_distance;
    let me = me.position();
    if let Some(id) = sim.players[&player].opened
        && (sim.entity(id).is_none() || !in_reach(sim, me, id, reach))
    {
        sim.players.get_mut(&player).unwrap().opened = None;
    }
    // The character's main inventory is kept sorted, as in Factorio.
    let db = sim.db.clone();
    character_mut(sim, player).inventory.sort_and_merge(&db);
}

/// cos(45°) in 16-bit fixed point.
const DIAGONAL: Fixed = Fixed::from_raw(46341);

fn walk(sim: &mut Simulation, player: u16) {
    let db = sim.db.clone();
    let c = sim.players[&player].character.as_ref().unwrap();
    let Some(dir) = c.walking else { return };
    let st = stats(&db, c.proto);
    let modifier =
        sim.surface.tile(c.position().tile()).map(|t| db.tile(t).walking_speed_modifier).unwrap_or(Fixed::ONE);
    let speed = st.running_speed * modifier;
    let (sx, sy): (i64, i64) = match dir.0 {
        0 => (0, -1),
        2 => (1, -1),
        4 => (1, 0),
        6 => (1, 1),
        8 => (0, 1),
        10 => (-1, 1),
        12 => (-1, 0),
        14 => (-1, -1),
        _ => (0, 0),
    };
    let diag = sx != 0 && sy != 0;
    let step = if diag { speed * DIAGONAL } else { speed };
    // Positions are whole 1/256 tiles, so each step is truncated to that grid: 0.15 tiles
    // per tick becomes 38/256, i.e. 8.9 tiles/s as in the game.
    let quantise = |v: Fixed| {
        let sub = (v.raw() * SUBTILES_PER_TILE as i64) >> Fixed::FRAC_BITS;
        Fixed::from_ratio(sub, SUBTILES_PER_TILE as i64)
    };
    let (dx, dy) = (quantise(step).mul_int(sx), quantise(step).mul_int(sy));
    let proto = c.proto;
    let (x, y) = (c.x, c.y);
    // Try the full move, then slide along each axis.
    for (nx, ny) in [(x + dx, y + dy), (x + dx, y), (x, y + dy)] {
        if (nx, ny) == (x, y) {
            continue;
        }
        if character_fits(sim, proto, Simulation::fixed_to_map(nx, ny)) {
            let c = character_mut(sim, player);
            c.x = nx;
            c.y = ny;
            return;
        }
    }
}

fn mine(sim: &mut Simulation, player: u16) {
    let db = sim.db.clone();
    let c = sim.players[&player].character.as_ref().unwrap();
    let Some(m) = c.mining.clone() else { return };
    let st = stats(&db, c.proto);
    let me = c.position();

    let (mining_ticks, results, valid) = match &m.target {
        MiningTarget::Entity(id) => match sim.entities.get(id) {
            Some(e) => {
                let p = db.entity(e.proto);
                let ok = in_reach(sim, me, *id, st.reach_distance);
                match &p.minable {
                    Some(mn) => (mn.mining_ticks, mn.results.clone(), ok),
                    None => (Fixed::ZERO, Vec::new(), false),
                }
            }
            None => (Fixed::ZERO, Vec::new(), false),
        },
        MiningTarget::Resource(t) => match sim.surface.resource(*t) {
            Some(r) => {
                let p = db.entity(r.proto);
                let area = p.collision_box.at(MapPosition::tile_center(*t));
                let category_ok =
                    matches!(&p.data, EntityData::Resource { category, .. } if st.mining_categories.contains(category));
                let ok = category_ok && distance_to_area(me, &area) <= st.reach_resource_distance;
                match &p.minable {
                    Some(mn) if mn.required_fluid.is_none() => (mn.mining_ticks, mn.results.clone(), ok),
                    _ => (Fixed::ZERO, Vec::new(), false),
                }
            }
            None => (Fixed::ZERO, Vec::new(), false),
        },
    };
    if !valid {
        character_mut(sim, player).mining = None;
        return;
    }

    // Results must fit before mining progresses.
    let c = character_mut(sim, player);
    for r in &results {
        if let (ItemOrFluid::Item(i), Some(n)) = (r.what, r.fixed_count())
            && c.inventory.space_for(&db, i) < n
        {
            return;
        }
    }
    let m = c.mining.as_mut().unwrap();
    m.progress += st.mining_speed;
    if m.progress < mining_ticks {
        return;
    }
    m.progress -= mining_ticks;

    match m.target.clone() {
        MiningTarget::Resource(t) => {
            let infinite = sim
                .surface
                .resource(t)
                .is_some_and(|r| matches!(db.entity(r.proto).data, EntityData::Resource { infinite: true, .. }));
            sim.surface.deplete(t, 1, infinite);
            if sim.surface.resource(t).is_none() {
                character_mut(sim, player).mining = None;
            }
        }
        MiningTarget::Entity(id) => {
            let contents = sim.remove_entity(id);
            for s in contents {
                give(sim, player, s.item, s.count);
            }
            character_mut(sim, player).mining = None;
        }
    }
    for r in results {
        if let (ItemOrFluid::Item(i), Some(n)) = (r.what, r.fixed_count()) {
            give(sim, player, i, n);
        }
    }
}

fn craft(sim: &mut Simulation, player: u16) {
    let db = sim.db.clone();
    let c = character_mut(sim, player);
    let Some(job) = c.queue.first() else { return };
    let recipe = db.recipe(job.recipe);
    c.craft_progress += Fixed::ONE;
    if c.craft_progress < recipe.ticks() {
        return;
    }
    c.craft_progress -= recipe.ticks();
    let job = c.queue.first_mut().unwrap();
    job.count -= 1;
    // A finished craft's ingredients are spent; only later crafts can be refunded.
    let per_craft: Vec<ItemStack> = recipe
        .ingredients
        .iter()
        .filter_map(|i| match i.what {
            ItemOrFluid::Item(item) => Some(ItemStack::new(item, i.amount.floor_int() as u32)),
            _ => None,
        })
        .collect();
    for s in &per_craft {
        if let Some(c) = job.consumed.iter_mut().find(|c| c.item == s.item) {
            c.count = c.count.saturating_sub(s.count);
        }
    }
    let mut outputs = Vec::new();
    for p in &recipe.results {
        if let (ItemOrFluid::Item(i), Some(mut n)) = (p.what, p.fixed_count()) {
            let keep = n.min(job.reserved);
            job.reserved -= keep;
            n -= keep;
            if keep > 0 {
                // Hand the reserved items to the job that needs them as refundable stock.
                outputs.push((i, 0, keep));
            }
            outputs.push((i, n, 0));
        }
    }
    let done = job.count == 0;
    if done {
        c.queue.remove(0);
        c.craft_progress = Fixed::ZERO;
    }
    for (item, give_n, reserved) in outputs {
        if reserved > 0 {
            let c = character_mut(sim, player);
            if let Some(next) = c
                .queue
                .iter_mut()
                .find(|j| db.recipe(j.recipe).ingredients.iter().any(|g| g.what == ItemOrFluid::Item(item)))
            {
                match next.consumed.iter_mut().find(|s| s.item == item) {
                    Some(s) => s.count += reserved,
                    None => next.consumed.push(ItemStack::new(item, reserved)),
                }
            }
        }
        if give_n > 0 {
            give(sim, player, item, give_n);
        }
    }
}
