//! Mining drills, crafting machines (furnaces and assemblers) and inserters.

use crate::energy::EnergyState;
use crate::fixed::Fixed;
use crate::inventory::{Inventory, ItemStack};
use crate::map::{Direction, MapPosition, SUBTILES_PER_TILE, TilePosition};
use crate::proto::{EnergySource, EntityData, EntityProto, ItemId, ItemOrFluid, PrototypeDb, RecipeId};
use crate::research::{Research, TriggerEvent};
use crate::world::{EntityId, EntityState, InsertSource, Simulation};

pub(crate) fn update_entity(sim: &mut Simulation, id: EntityId) {
    let Some(e) = sim.entities.get_mut(&id) else { return };
    match e.state {
        EntityState::Drill(_) => update_drill(sim, id),
        EntityState::Crafter(_) => update_crafter(sim, id),
        EntityState::Inserter(_) => update_inserter(sim, id),
        EntityState::Lab(_) => crate::research::update_lab(sim, id),
        _ => {}
    }
}

/// Takes an entity's state out for the duration of its update, so other entities can be
/// mutated meanwhile. Returns the entity's prototype, position and direction too.
fn take_state(
    sim: &mut Simulation,
    id: EntityId,
) -> (EntityState, crate::proto::EntityProtoId, MapPosition, Direction) {
    let e = sim.entities.get_mut(&id).unwrap();
    (std::mem::take(&mut e.state), e.proto, e.position, e.direction)
}

fn put_state(sim: &mut Simulation, id: EntityId, state: EntityState) {
    if let Some(e) = sim.entities.get_mut(&id) {
        e.state = state;
    }
}

fn fixed_to_subtiles(v: Fixed) -> i32 {
    ((v.raw() * SUBTILES_PER_TILE as i64) >> Fixed::FRAC_BITS) as i32
}

/// Fraction of the requested energy actually obtained, 0..=1.
fn energy_fraction(got: Fixed, need: Fixed) -> Fixed {
    if !need.is_positive() || got >= need { Fixed::ONE } else { got / need }
}

/// Places one item at a point: onto a belt lane, into an entity, or onto the ground.
fn output_item(
    sim: &mut Simulation,
    at: MapPosition,
    item: ItemId,
    source: InsertSource,
    from: MapPosition,
    far_lane: bool,
) -> bool {
    let tile = at.tile();
    if let Some(bid) = sim.belts.at_tile(tile) {
        let b = sim.belts.get_mut(bid).unwrap();
        let (lane, pos) = b.lane_at_point(tile, at);
        let lane = if far_lane { b.lanes_seen_from(tile, from).1 } else { lane };
        return b.try_insert(lane, pos, item);
    }
    if let Some(eid) = sim.entity_at(at) {
        return sim.insert_into_entity(eid, item, 1, source) == 1;
    }
    sim.drop_on_ground(at, item)
}

// ----- mining drills -----

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct DrillState {
    pub energy: EnergyState,
    /// Accumulated mining speed; a resource is mined when it reaches `mining_time * 60`.
    pub progress: Fixed,
    /// Mined item waiting for space at the output.
    pub output: Option<ItemStack>,
    pub working: bool,
    /// Mining productivity bar, 0..1.
    pub productivity: Fixed,
}

impl DrillState {
    pub fn new(source: &EnergySource) -> Self {
        DrillState { energy: EnergyState::for_source(source), ..Default::default() }
    }
}

/// Resource tiles a drill at this spot can mine, in row-major order.
pub fn drill_resources(
    sim: &Simulation,
    proto: &EntityProto,
    position: MapPosition,
    _dir: Direction,
) -> Vec<TilePosition> {
    let EntityData::MiningDrill { radius, resource_categories, .. } = &proto.data else { return Vec::new() };
    let db = &sim.db;
    let r = *radius;
    let lt = MapPosition::new(position.x - r, position.y - r).tile();
    let rb = MapPosition::new(position.x + r, position.y + r).tile();
    let mut out = Vec::new();
    for y in lt.y..=rb.y {
        for x in lt.x..=rb.x {
            let t = TilePosition::new(x, y);
            let c = MapPosition::tile_center(t);
            if (c.x - position.x).abs() > r || (c.y - position.y).abs() > r {
                continue;
            }
            if let Some(res) = sim.surface.resource(t) {
                let rp = db.entity(res.proto);
                let ok = matches!(&rp.data, EntityData::Resource { category, .. } if resource_categories.contains(category))
                    && rp.minable.as_ref().is_some_and(|m| m.required_fluid.is_none());
                if ok {
                    out.push(t);
                }
            }
        }
    }
    out
}

fn update_drill(sim: &mut Simulation, id: EntityId) {
    let db = sim.db.clone();
    let (state, proto_id, position, direction) = take_state(sim, id);
    let EntityState::Drill(mut d) = state else { unreachable!() };
    let proto = db.entity(proto_id);
    let EntityData::MiningDrill { mining_speed, energy_usage, energy_source, output_vector, .. } = &proto.data else {
        unreachable!()
    };
    let [ox, oy] = direction.rotate_vec(*output_vector);
    let out_at = position.offset(ox, oy);

    if let Some(stack) = d.output {
        if output_item(sim, out_at, stack.item, InsertSource::Player, position, false) {
            d.output = None;
        } else {
            d.working = false;
            put_state(sim, id, EntityState::Drill(d));
            return;
        }
    }

    let resources = drill_resources(sim, proto, position, direction);
    let Some(&target) = resources.first() else {
        d.working = false;
        put_state(sim, id, EntityState::Drill(d));
        return;
    };
    let got = d.energy.draw(&db, energy_source, *energy_usage);
    let frac = energy_fraction(got, *energy_usage);
    d.working = frac.is_positive();
    d.progress += *mining_speed * frac;

    let res_proto = sim.surface.resource(target).unwrap().proto;
    let rp = db.entity(res_proto);
    let minable = rp.minable.as_ref().unwrap();
    let threshold = minable.mining_ticks;
    if d.progress >= threshold {
        d.progress -= threshold;
        let infinite = matches!(rp.data, EntityData::Resource { infinite: true, .. });
        sim.surface.deplete(target, 1, infinite);
        sim.research_trigger(TriggerEvent::Mined(res_proto));
        // Mining productivity fills a bar; each time it is full an extra result is made.
        d.productivity += sim.research.modifier("mining-drill-productivity-bonus", None);
        let extra = if d.productivity >= Fixed::ONE {
            d.productivity -= Fixed::ONE;
            1
        } else {
            0
        };
        if let Some(item) = minable.results.iter().find_map(|r| match (r.what, r.fixed_count()) {
            (ItemOrFluid::Item(i), Some(n)) => Some(ItemStack::new(i, n + extra)),
            _ => None,
        }) {
            d.output = Some(item);
            if output_item(sim, out_at, item.item, InsertSource::Player, position, false) {
                if item.count > 1 {
                    d.output = Some(ItemStack::new(item.item, item.count - 1));
                } else {
                    d.output = None;
                }
            }
        }
    }
    put_state(sim, id, EntityState::Drill(d));
}

// ----- crafting machines -----

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CrafterState {
    pub energy: EnergyState,
    pub recipe: Option<RecipeId>,
    pub input: Inventory,
    pub output: Inventory,
    /// Crafting progress in ticks at crafting speed 1.
    pub progress: Fixed,
    /// Ingredients for the current craft have been consumed.
    pub crafting: bool,
    pub furnace: bool,
}

/// Ingredients for this many crafts may be inserted automatically: one craft plus what
/// completes during one 1.166 s inserter swing (rounded up), at least 2, at most 100.
pub fn automated_craft_limit(crafting_speed: Fixed, energy_required: Fixed) -> u32 {
    if !energy_required.is_positive() {
        return 100;
    }
    let during_swing = (Fixed::from_ratio(1166, 1000) * crafting_speed / energy_required).ceil_int().max(0) as u32;
    (1 + during_swing).clamp(2, 100)
}

impl CrafterState {
    pub fn new(proto: &EntityProto) -> Self {
        let EntityData::CraftingMachine {
            furnace,
            energy_source,
            source_inventory_size,
            result_inventory_size,
            fixed_recipe,
            ..
        } = &proto.data
        else {
            unreachable!()
        };
        CrafterState {
            energy: EnergyState::for_source(energy_source),
            recipe: *fixed_recipe,
            input: Inventory::new(source_inventory_size.unwrap_or(0)),
            output: Inventory::new(result_inventory_size.unwrap_or(0)),
            furnace: *furnace,
            ..Default::default()
        }
    }

    /// Changes the recipe; returns everything that was inside (and an unfinished craft's
    /// ingredients) so the caller can give it back.
    pub fn set_recipe(&mut self, db: &PrototypeDb, proto: &EntityProto, recipe: Option<RecipeId>) -> Vec<ItemStack> {
        if self.furnace || self.recipe == recipe {
            return Vec::new();
        }
        let EntityData::CraftingMachine { crafting_categories, .. } = &proto.data else { return Vec::new() };
        if let Some(r) = recipe {
            let rec = db.recipe(r);
            let items_only = rec.ingredients.iter().chain_results(rec);
            if !crafting_categories.contains(&rec.category) || !items_only {
                return Vec::new();
            }
        }
        let mut out = self.input.take_all();
        out.extend(self.output.take_all());
        if self.crafting
            && let Some(r) = self.recipe
        {
            out.extend(item_ingredients(db, r));
        }
        self.crafting = false;
        self.progress = Fixed::ZERO;
        self.recipe = recipe;
        match recipe {
            Some(r) => {
                let rec = db.recipe(r);
                self.input = Inventory::new(rec.ingredients.len() as u32);
                self.output = Inventory::new(rec.results.len() as u32);
            }
            None => {
                self.input = Inventory::new(0);
                self.output = Inventory::new(0);
            }
        }
        out
    }

    /// The enabled recipe a furnace uses for `item`.
    fn furnace_recipe_for(
        db: &PrototypeDb,
        research: &Research,
        proto: &EntityProto,
        item: ItemId,
    ) -> Option<RecipeId> {
        let EntityData::CraftingMachine { crafting_categories, .. } = &proto.data else { return None };
        db.recipe_ids().find(|r| {
            let rec = db.recipe(*r);
            research.recipe_enabled(*r)
                && crafting_categories.contains(&rec.category)
                && rec.ingredients.len() == 1
                && rec.ingredients[0].what == ItemOrFluid::Item(item)
        })
    }

    /// How many of `item` could be inserted as an ingredient, honouring automatic
    /// insertion limits for `Automated`.
    pub fn ingredient_room(
        &self,
        db: &PrototypeDb,
        research: &Research,
        proto: &EntityProto,
        item: ItemId,
        source: InsertSource,
    ) -> u32 {
        let EntityData::CraftingMachine { crafting_speed, .. } = &proto.data else { return 0 };
        let recipe = if self.furnace {
            if let Some(existing) = self.input.first_item()
                && existing != item
            {
                return 0;
            }
            match Self::furnace_recipe_for(db, research, proto, item) {
                Some(r) => r,
                None => return 0,
            }
        } else {
            match self.recipe {
                Some(r) => r,
                None => return 0,
            }
        };
        let rec = db.recipe(recipe);
        let Some(slot) = rec.ingredients.iter().position(|i| i.what == ItemOrFluid::Item(item)) else { return 0 };
        let per_craft = rec.ingredients[slot].amount.floor_int() as u32;
        let stack = db.item(item).stack_size;
        let have = if self.furnace { self.input.count(item) } else { self.input.slot(slot).map_or(0, |s| s.count) };
        let mut room = if self.furnace { self.input.space_for(db, item) } else { stack.saturating_sub(have) };
        if source == InsertSource::Automated {
            let limit = per_craft * automated_craft_limit(*crafting_speed, rec.energy_required);
            room = room.min(limit.saturating_sub(have));
        }
        room
    }

    /// Inserts recipe ingredients, honouring automatic insertion limits for `Automated`.
    pub fn insert_ingredient(
        &mut self,
        db: &PrototypeDb,
        research: &Research,
        proto: &EntityProto,
        item: ItemId,
        count: u32,
        source: InsertSource,
    ) -> u32 {
        let n = count.min(self.ingredient_room(db, research, proto, item, source));
        if n == 0 {
            return 0;
        }
        if self.furnace {
            self.input.insert(db, item, n)
        } else {
            let rec = db.recipe(self.recipe.unwrap());
            let slot = rec.ingredients.iter().position(|i| i.what == ItemOrFluid::Item(item)).unwrap();
            let have = self.input.slot(slot).map_or(0, |s| s.count);
            self.input.set_slot(slot, Some(ItemStack::new(item, have + n)));
            n
        }
    }

    fn has_ingredients(&self, db: &PrototypeDb, r: RecipeId) -> bool {
        db.recipe(r).ingredients.iter().all(|i| match i.what {
            ItemOrFluid::Item(item) => self.input.count(item) >= i.amount.floor_int() as u32,
            ItemOrFluid::Fluid(_) => false,
        })
    }

    fn results_fit(&self, db: &PrototypeDb, r: RecipeId) -> bool {
        db.recipe(r).results.iter().all(|p| match (p.what, p.fixed_count()) {
            (ItemOrFluid::Item(i), Some(n)) => self.output.space_for(db, i) >= n,
            (ItemOrFluid::Item(i), None) => self.output.space_for(db, i) >= p.amount_max.ceil_int() as u32,
            (ItemOrFluid::Fluid(_), _) => false,
        })
    }

    fn try_start(&mut self, db: &PrototypeDb) -> bool {
        let Some(r) = self.recipe else { return false };
        if !self.has_ingredients(db, r) || !self.results_fit(db, r) {
            return false;
        }
        for s in item_ingredients(db, r) {
            self.input.remove(s.item, s.count);
        }
        self.crafting = true;
        true
    }
}

trait ChainResults {
    fn chain_results(self, rec: &crate::proto::RecipeProto) -> bool;
}

impl<'a, I: Iterator<Item = &'a crate::proto::Ingredient>> ChainResults for I {
    /// True when every ingredient and result is an item (fluids are not simulated yet).
    fn chain_results(mut self, rec: &crate::proto::RecipeProto) -> bool {
        self.all(|i| matches!(i.what, ItemOrFluid::Item(_)))
            && rec.results.iter().all(|p| matches!(p.what, ItemOrFluid::Item(_)))
    }
}

fn item_ingredients(db: &PrototypeDb, r: RecipeId) -> Vec<ItemStack> {
    db.recipe(r)
        .ingredients
        .iter()
        .filter_map(|i| match i.what {
            ItemOrFluid::Item(item) => Some(ItemStack::new(item, i.amount.floor_int() as u32)),
            _ => None,
        })
        .collect()
}

fn update_crafter(sim: &mut Simulation, id: EntityId) {
    let db = sim.db.clone();
    let (state, proto_id, _, _) = take_state(sim, id);
    let EntityState::Crafter(mut c) = state else { unreachable!() };
    let proto = db.entity(proto_id);
    let EntityData::CraftingMachine { crafting_speed, energy_usage, energy_source, .. } = &proto.data else {
        unreachable!()
    };

    if let EnergySource::Electric { drain, .. } = energy_source {
        c.energy.draw(&db, energy_source, *drain);
    }
    if c.furnace && !c.crafting {
        c.recipe = c
            .input
            .first_item()
            .and_then(|i| CrafterState::furnace_recipe_for(&db, &sim.research, proto, i))
            .or(c.recipe);
    }
    if !c.crafting && !c.try_start(&db) {
        c.progress = Fixed::ZERO;
        put_state(sim, id, EntityState::Crafter(c));
        return;
    }
    let got = c.energy.draw(&db, energy_source, *energy_usage);
    c.progress += *crafting_speed * energy_fraction(got, *energy_usage);
    let r = c.recipe.unwrap();
    let ticks = db.recipe(r).ticks();
    let mut crafted = Vec::new();
    if c.progress >= ticks && c.results_fit(&db, r) {
        c.progress -= ticks;
        for p in &db.recipe(r).results {
            if let ItemOrFluid::Item(i) = p.what {
                let n = match p.fixed_count() {
                    Some(n) => n,
                    None => {
                        // Probabilistic or ranged results draw from the map RNG.
                        let lo = p.amount_min.floor_int() as u32;
                        let hi = p.amount_max.floor_int() as u32;
                        let amount = lo + sim.rng.below(hi - lo + 1);
                        let roll = Fixed::from_raw(sim.rng.below(1 << 16) as i64);
                        if roll < p.probability { amount } else { 0 }
                    }
                };
                c.output.insert(&db, i, n);
                crafted.push((i, n));
            }
        }
        c.crafting = false;
        if c.furnace {
            c.recipe = c
                .input
                .first_item()
                .and_then(|i| CrafterState::furnace_recipe_for(&db, &sim.research, proto, i))
                .or(c.recipe);
        }
        if !c.try_start(&db) {
            c.progress = Fixed::ZERO;
        }
    }
    put_state(sim, id, EntityState::Crafter(c));
    for (item, n) in crafted {
        if n > 0 {
            sim.research_trigger(TriggerEvent::Crafted(item, n));
        }
    }
}

// ----- inserters -----

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct InserterState {
    pub energy: EnergyState,
    pub hand: Option<ItemStack>,
    /// Item filters (as many as the prototype's `filter_count`), used when `use_filters`.
    pub filters: Vec<Option<ItemId>>,
    pub use_filters: bool,
    /// The filters list what not to take, instead of what to take.
    pub blacklist: bool,
    /// A smaller hand size set by the player (the window's "Override stack size").
    pub stack_override: Option<u32>,
    /// Arm angle in turns: 0 at the pickup position, 0.5 at the drop position.
    pub rotation: Fixed,
    /// Arm length in tiles.
    pub extension: Fixed,
}

fn vec_len(v: [Fixed; 2]) -> Fixed {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

impl InserterState {
    pub fn new(proto: &EntityProto, source: &EnergySource) -> Self {
        let EntityData::Inserter { pickup_position, .. } = &proto.data else { unreachable!() };
        let EntityData::Inserter { filter_count, .. } = &proto.data else { unreachable!() };
        InserterState {
            energy: EnergyState::for_source(source),
            extension: vec_len(*pickup_position),
            filters: vec![None; *filter_count as usize],
            ..Default::default()
        }
    }

    /// Whether the filters let this inserter move `item`.
    pub fn allows(&self, item: ItemId) -> bool {
        if !self.use_filters || self.filters.iter().all(Option::is_none) {
            return true;
        }
        self.filters.contains(&Some(item)) != self.blacklist
    }

    pub fn reset_after_rotation(&mut self) {
        // Rotating keeps the arm where it is; it simply has new targets.
    }
}

/// Moves `value` towards `target` by at most `speed`, arriving early when the remaining
/// distance after the step would be less than one more step (Factorio's inserter timing:
/// a half turn at 0.014 turns/tick takes 35 ticks, at 0.013 takes 38).
fn approach(value: Fixed, target: Fixed, speed: Fixed) -> (Fixed, bool) {
    let diff = target - value;
    if diff.abs() <= speed {
        return (target, true);
    }
    let next = if diff.is_positive() { value + speed } else { value - speed };
    if (target - next).abs() < speed { (target, true) } else { (next, false) }
}

fn update_inserter(sim: &mut Simulation, id: EntityId) {
    let db = sim.db.clone();
    let (state, proto_id, position, direction) = take_state(sim, id);
    let EntityState::Inserter(mut ins) = state else { unreachable!() };
    let proto = db.entity(proto_id);
    let EntityData::Inserter {
        rotation_speed,
        extension_speed,
        pickup_position,
        insert_position,
        energy_per_movement,
        energy_per_rotation,
        energy_source,
        ..
    } = &proto.data
    else {
        unreachable!()
    };

    let rot = |v: [Fixed; 2]| {
        let [x, y] = direction.rotate_vec([fixed_to_subtiles(v[0]), fixed_to_subtiles(v[1])]);
        position.offset(x, y)
    };
    let pickup_at = rot(*pickup_position);
    let drop_at = rot(*insert_position);
    let pickup_len = vec_len(*pickup_position);
    let drop_len = vec_len(*insert_position);

    if let EnergySource::Electric { drain, .. } = energy_source {
        ins.energy.draw(&db, energy_source, *drain);
    }

    let half = Fixed::from_ratio(1, 2);
    let at_pickup = |ins: &InserterState| ins.rotation == Fixed::ZERO && ins.extension == pickup_len;
    let at_drop = |ins: &InserterState| ins.rotation == half && ins.extension == drop_len;

    // Act if the hand is already where it needs to be (e.g. waiting for items or space).
    let acted = match ins.hand {
        None if at_pickup(&ins) => {
            pick_up(sim, &db, &mut ins, proto, pickup_at, drop_at, position);
            true
        }
        // Still at the pickup with room in the hand: keep taking while items are there
        // (from belts one per tick), leaving when none came unless it waits for a full hand.
        Some(stack) if at_pickup(&ins) && stack.count < hand_limit(sim, proto, &ins, drop_at, stack.item) => {
            let before = stack.count;
            pick_up(sim, &db, &mut ins, proto, pickup_at, drop_at, position);
            let got = ins.hand.map_or(0, |h| h.count) > before;
            let waits = matches!(proto.data, EntityData::Inserter { wait_for_full_hand: true, .. });
            got || waits
        }
        Some(stack) if at_drop(&ins) => {
            drop_item(sim, &mut ins, stack, drop_at, position);
            true
        }
        _ => false,
    };
    if !acted {
        // Move towards the next target, scaled by available energy. The pickup or drop
        // happens in the same tick the hand arrives.
        let (target_rot, target_ext) = if ins.hand.is_some() { (half, drop_len) } else { (Fixed::ZERO, pickup_len) };
        let moving_ext = ins.extension != target_ext;
        let need = *rotation_speed * *energy_per_rotation
            + if moving_ext { *extension_speed * *energy_per_movement } else { Fixed::ZERO };
        let got = ins.energy.draw(&db, energy_source, need);
        let f = energy_fraction(got, need);
        if f.is_positive() {
            ins.rotation = approach(ins.rotation, target_rot, *rotation_speed * f).0;
            ins.extension = approach(ins.extension, target_ext, *extension_speed * f).0;
            match ins.hand {
                None if at_pickup(&ins) => pick_up(sim, &db, &mut ins, proto, pickup_at, drop_at, position),
                Some(stack) if at_drop(&ins) => drop_item(sim, &mut ins, stack, drop_at, position),
                _ => {}
            }
        }
    }
    put_state(sim, id, EntityState::Inserter(ins));
}

/// How many items the hand holds at most: 1, plus the prototype's bonus, plus the force's
/// inserter stack size bonus (or bulk inserter capacity bonus for bulk inserters).
pub fn hand_size(sim: &Simulation, proto: &EntityProto) -> u32 {
    let EntityData::Inserter { bulk, stack_size_bonus, uses_stack_size_bonus, .. } = &proto.data else { return 1 };
    let research = if *uses_stack_size_bonus {
        let kind = if *bulk { "bulk-inserter-capacity-bonus" } else { "inserter-stack-size-bonus" };
        sim.research.modifier(kind, None).floor_int().max(0) as u32
    } else {
        0
    };
    1 + stack_size_bonus + research
}

/// The hand size of this inserter, with its override.
pub fn effective_hand_size(sim: &Simulation, proto: &EntityProto, ins: &InserterState) -> u32 {
    let size = hand_size(sim, proto);
    ins.stack_override.map_or(size, |o| o.clamp(1, size))
}

/// The most this inserter should hold of `item` for its drop target: its hand size, but
/// no more than a machine or chest can take (belts and the ground take any amount).
fn hand_limit(sim: &Simulation, proto: &EntityProto, ins: &InserterState, drop_at: MapPosition, item: ItemId) -> u32 {
    let size = effective_hand_size(sim, proto, ins);
    let needs_fuel = ins.energy.burner().is_some_and(|b| !b.has_fuel());
    if sim.belts.at_tile(drop_at.tile()).is_some() || needs_fuel {
        return size;
    }
    match sim.entity_at(drop_at) {
        Some(eid) => size.min(sim.inserter_room(eid, item).max(1)),
        None => size,
    }
}

fn drop_item(
    sim: &mut Simulation,
    ins: &mut InserterState,
    stack: ItemStack,
    drop_at: MapPosition,
    position: MapPosition,
) {
    // Onto a belt or the ground one item per tick; into an entity as many as fit at once.
    let onto_belt = sim.belts.at_tile(drop_at.tile()).is_some();
    let into_entity = !onto_belt && sim.entity_at(drop_at).is_some();
    let mut left = stack.count;
    while left > 0 && output_item(sim, drop_at, stack.item, InsertSource::Automated, position, true) {
        left -= 1;
        if !into_entity {
            break;
        }
    }
    ins.hand = (left > 0).then(|| ItemStack::new(stack.item, left));
}

/// Takes items at the pickup position: with an empty hand, one stack of an acceptable
/// item (all at once from an entity, one item from a belt or the ground); with items in
/// the hand, more of the same.
fn pick_up(
    sim: &mut Simulation,
    db: &PrototypeDb,
    ins: &mut InserterState,
    proto: &EntityProto,
    pickup_at: MapPosition,
    drop_at: MapPosition,
    position: MapPosition,
) {
    // A burner inserter with no fuel takes fuel for itself first.
    let needs_own_fuel = ins.energy.burner().is_some_and(|b| !b.has_fuel()) && ins.hand.is_none();
    let source = proto.energy_source().cloned();
    let fuel_for_self = |i: ItemId| match &source {
        Some(src) => crate::energy::Burner::accepts(db, src, i),
        None => false,
    };

    let drop_entity = if sim.belts.at_tile(drop_at.tile()).is_some() { None } else { sim.entity_at(drop_at) };

    // Decide which of the items available at the source are acceptable.
    let tile = pickup_at.tile();
    let belt = sim.belts.at_tile(tile);
    let source_entity = if belt.is_some() { None } else { sim.entity_at(pickup_at) };
    let held = ins.hand.map(|h| h.item);
    let mut candidates: Vec<ItemId> = match (belt, source_entity) {
        (Some(bid), _) => {
            sim.belts.get(bid).unwrap().lanes.iter().flat_map(|l| l.items.iter().map(|i| i.item)).collect()
        }
        (None, Some(eid)) => sim.items_for_inserter(eid),
        (None, None) => sim.ground_items_near(pickup_at),
    };
    candidates.sort();
    candidates.dedup();
    let accepted: Vec<ItemId> = candidates
        .into_iter()
        .filter(|i| held.is_none_or(|h| h == *i))
        .filter(|i| {
            (needs_own_fuel && fuel_for_self(*i))
                || (ins.allows(*i)
                    && match drop_entity {
                        Some(eid) => sim.entity_wants(eid, *i),
                        None => true,
                    })
        })
        .collect();
    if accepted.is_empty() {
        return;
    }
    let accept = |i: ItemId| accepted.binary_search(&i).is_ok();
    let take_one = |sim: &mut Simulation, accept: &dyn Fn(ItemId) -> bool| match (belt, source_entity) {
        (Some(bid), _) => {
            let b = sim.belts.get_mut(bid).unwrap();
            let (near, far) = b.lanes_seen_from(tile, position);
            b.take(&[near, far], accept)
        }
        (None, Some(eid)) => sim.take_for_inserter(eid, accept),
        (None, None) => sim.take_from_ground(pickup_at, accept),
    };
    let Some(item) = take_one(sim, &accept) else { return };
    if needs_own_fuel && fuel_for_self(item) {
        let b = ins.energy.burner_mut().unwrap();
        b.fuel.insert(db, item, 1);
        return;
    }
    let mut count = ins.hand.map_or(0, |h| h.count) + 1;
    // From an entity, the rest of the hand in the same tick.
    if source_entity.is_some() {
        let limit = hand_limit(sim, proto, ins, drop_at, item);
        while count < limit {
            if take_one(sim, &|i: ItemId| i == item).is_none() {
                break;
            }
            count += 1;
        }
    }
    ins.hand = Some(ItemStack::new(item, count));
}
