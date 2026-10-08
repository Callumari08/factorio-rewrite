//! Research: technologies, the research queue, labs, triggers, recipe unlocks and bonuses.
//!
//! There is one force (Factorio's `player` force), shared by every player.
//!
//! Lab work is counted exactly: each lab adds its speed (as a raw [`Fixed`]) to the current
//! technology every tick, and a level is finished when the total reaches
//! `count * time_ticks * Fixed::ONE`. So N identical labs finish exactly N times faster,
//! with no rounding drift, like the game's continuous research progress.

use std::collections::BTreeMap;

use crate::energy::EnergyState;
use crate::fixed::Fixed;
use crate::inventory::{Inventory, ItemStack};
use crate::proto::{
    EnergySource, EntityData, EntityProto, EntityProtoId, FluidId, ItemId, PrototypeDb, RecipeId, ResearchTrigger,
    TechEffect, TechId,
};
use crate::world::{EntityId, EntityState, Simulation};

/// Factorio's limit on the number of technologies in the research queue.
pub const MAX_QUEUE: usize = 7;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Research {
    /// Fully researched (every level, for leveled technologies).
    pub researched: Vec<bool>,
    /// The level being researched next.
    pub level: Vec<u32>,
    /// Lab work done on the current level; see the module docs.
    pub progress: Vec<u128>,
    /// Progress towards each technology's research trigger.
    pub trigger_counts: Vec<u64>,
    /// Technologies waiting for labs, the first being researched now.
    pub queue: Vec<TechId>,
    /// Which recipes the force may use.
    pub recipes: Vec<bool>,
    /// Summed modifiers by effect type and qualifier (ammo category, turret id, ...).
    pub modifiers: BTreeMap<(String, String), Fixed>,
    /// The most recently finished technology and the tick it finished on, for the GUI.
    pub last_finished: Option<(TechId, u64)>,
}

/// Something that happened in the world that a research trigger may be waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerEvent {
    Crafted(ItemId, u32),
    Mined(EntityProtoId),
    Built(EntityProtoId),
    CraftedFluid(FluidId, Fixed),
}

/// Why a technology could not be queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueError {
    AlreadyResearched,
    Disabled,
    /// It, or a missing prerequisite, is researched by a trigger rather than labs.
    NeedsTrigger(TechId),
    QueueFull,
}

impl Research {
    pub fn new(db: &PrototypeDb) -> Self {
        let n = db.technologies.len();
        Research {
            researched: vec![false; n],
            level: db.technologies.iter().map(|t| t.level).collect(),
            progress: vec![0; n],
            trigger_counts: vec![0; n],
            queue: Vec::new(),
            recipes: db.recipes.iter().map(|r| r.enabled).collect(),
            modifiers: BTreeMap::new(),
            last_finished: None,
        }
    }

    pub fn recipe_enabled(&self, r: RecipeId) -> bool {
        self.recipes.get(r.index()).copied().unwrap_or(true)
    }

    pub fn is_researched(&self, t: TechId) -> bool {
        self.researched[t.index()]
    }

    /// Researchable now: enabled, not done, every prerequisite done.
    pub fn is_available(&self, db: &PrototypeDb, t: TechId) -> bool {
        let tech = db.technology(t);
        tech.enabled && !self.researched[t.index()] && tech.prerequisites.iter().all(|p| self.researched[p.index()])
    }

    /// The technology labs are working on.
    pub fn current(&self) -> Option<TechId> {
        self.queue.first().copied()
    }

    /// Sum of all modifiers of this effect type (any qualifier when `qualifier` is `None`).
    pub fn modifier(&self, kind: &str, qualifier: Option<&str>) -> Fixed {
        self.modifiers
            .iter()
            .filter(|((k, q), _)| k == kind && qualifier.is_none_or(|w| w == q))
            .fold(Fixed::ZERO, |acc, (_, v)| acc + *v)
    }

    /// Lab work needed for the technology's current level.
    pub fn work_needed(&self, db: &PrototypeDb, t: TechId) -> u128 {
        let Some(unit) = &db.technology(t).unit else { return 0 };
        unit.count_for(self.level[t.index()]) as u128 * unit.time_ticks.max(1) as u128 * Fixed::ONE.raw() as u128
    }

    /// Progress on the current level, 0..=1, for display.
    pub fn progress_fraction(&self, db: &PrototypeDb, t: TechId) -> Fixed {
        let need = self.work_needed(db, t);
        if need == 0 {
            return Fixed::ZERO;
        }
        Fixed::from_raw(((self.progress[t.index()].min(need) << Fixed::FRAC_BITS) / need) as i64)
    }

    /// Prerequisites still missing, deepest first, followed by `t` itself.
    fn missing_chain(&self, db: &PrototypeDb, t: TechId, out: &mut Vec<TechId>) {
        if self.researched[t.index()] || out.contains(&t) {
            return;
        }
        for p in &db.technology(t).prerequisites {
            self.missing_chain(db, *p, out);
        }
        out.push(t);
    }

    /// The first technology in `t`'s missing prerequisite chain (or `t` itself) that labs
    /// cannot research because it needs a trigger.
    pub fn blocking_trigger(&self, db: &PrototypeDb, t: TechId) -> Option<TechId> {
        let mut chain = Vec::new();
        self.missing_chain(db, t, &mut chain);
        chain.into_iter().find(|c| db.technology(*c).unit.is_none())
    }

    /// Adds a technology to the end (or the front) of the queue, with any prerequisites
    /// that are not researched or queued yet, as the game does.
    pub fn enqueue(&mut self, db: &PrototypeDb, t: TechId, front: bool) -> Result<(), QueueError> {
        if self.researched[t.index()] {
            return Err(QueueError::AlreadyResearched);
        }
        let mut chain = Vec::new();
        self.missing_chain(db, t, &mut chain);
        for c in &chain {
            let tech = db.technology(*c);
            if !tech.enabled {
                return Err(QueueError::Disabled);
            }
            if tech.unit.is_none() {
                return Err(QueueError::NeedsTrigger(*c));
            }
        }
        if front {
            // Moving to the front also pulls its prerequisites forward.
            self.queue.retain(|q| !chain.contains(q));
            if self.queue.len() + chain.len() > MAX_QUEUE {
                return Err(QueueError::QueueFull);
            }
            self.queue.splice(0..0, chain);
        } else {
            chain.retain(|c| !self.queue.contains(c));
            if self.queue.len() + chain.len() > MAX_QUEUE {
                return Err(QueueError::QueueFull);
            }
            self.queue.extend(chain);
        }
        Ok(())
    }

    /// Removes a technology and everything queued that depends on it.
    pub fn dequeue(&mut self, db: &PrototypeDb, t: TechId) {
        let mut removed = vec![t];
        let mut changed = true;
        while changed {
            changed = false;
            for q in &self.queue {
                if !removed.contains(q) && db.technology(*q).prerequisites.iter().any(|p| removed.contains(p)) {
                    removed.push(*q);
                    changed = true;
                }
            }
        }
        self.queue.retain(|q| !removed.contains(q));
    }
}

impl Simulation {
    pub fn research(&self) -> &Research {
        &self.research
    }

    /// Finishes the current level of a technology and applies its effects.
    pub fn finish_research(&mut self, t: TechId) {
        let db = self.db.clone();
        let tech = db.technology(t);
        let r = &mut self.research;
        if r.researched[t.index()] {
            return;
        }
        let level = r.level[t.index()];
        r.progress[t.index()] = 0;
        if tech.max_level.is_some_and(|m| level >= m) {
            r.researched[t.index()] = true;
        } else {
            r.level[t.index()] = level + 1;
        }
        r.queue.retain(|q| *q != t);
        r.last_finished = Some((t, self.tick));
        self.events.push(crate::world::GameEvent::ResearchFinished(t));
        let mut gifts = Vec::new();
        for e in &tech.effects {
            match e {
                TechEffect::UnlockRecipe(rec) => r.recipes[rec.index()] = true,
                TechEffect::GiveItem { item, count } => gifts.push(ItemStack::new(*item, *count)),
                TechEffect::Modifier { kind, qualifier, modifier } => {
                    *r.modifiers.entry((kind.clone(), qualifier.clone())).or_insert(Fixed::ZERO) += *modifier;
                }
            }
        }
        crate::player::apply_research_bonuses(self);
        let players: Vec<u16> = self.players.keys().copied().collect();
        for s in gifts {
            for p in &players {
                crate::player::give_or_spill(self, *p, s.item, s.count);
            }
        }
    }

    /// Researches everything (Factorio's `/cheat all` research part).
    pub fn research_all(&mut self) {
        let db = self.db.clone();
        loop {
            let next = db
                .technology_ids()
                .find(|t| self.research.is_available(&db, *t) && db.technology(*t).max_level.is_some());
            match next {
                Some(t) => self.finish_research(t),
                None => break,
            }
        }
    }

    /// Reports an event to research triggers; finishes any trigger technology it completes.
    pub fn research_trigger(&mut self, event: TriggerEvent) {
        let db = self.db.clone();
        let mut done = Vec::new();
        for t in db.technology_ids() {
            let Some(trigger) = &db.technology(t).trigger else { continue };
            let add: u64 = match (trigger, event) {
                (ResearchTrigger::CraftItem { item, .. }, TriggerEvent::Crafted(i, n)) if *item == i => n as u64,
                (ResearchTrigger::MineEntity { entity }, TriggerEvent::Mined(e)) if *entity == e => 1,
                (ResearchTrigger::BuildEntity { entity }, TriggerEvent::Built(e)) if *entity == e => 1,
                (ResearchTrigger::CraftFluid { fluid, .. }, TriggerEvent::CraftedFluid(f, a)) if *fluid == f => {
                    a.raw().max(0) as u64
                }
                _ => continue,
            };
            // Triggers only count while the technology is researchable.
            if !self.research.is_available(&db, t) {
                continue;
            }
            let count = &mut self.research.trigger_counts[t.index()];
            *count += add;
            let needed = match trigger {
                ResearchTrigger::CraftItem { count, .. } => *count as u64,
                ResearchTrigger::CraftFluid { amount, .. } => amount.raw().max(1) as u64,
                _ => 1,
            };
            if *count >= needed {
                done.push(t);
            }
        }
        for t in done {
            self.finish_research(t);
        }
    }

    /// Adds lab work to the current research.
    fn add_research_work(&mut self, work: u128) {
        let Some(t) = self.research.current() else { return };
        let db = self.db.clone();
        let r = &mut self.research;
        r.progress[t.index()] += work;
        if r.progress[t.index()] >= r.work_needed(&db, t) {
            self.finish_research(t);
        }
    }
}

// ----- labs -----

/// Durability is tracked in units of 2^-48 of a pack, so per-tick drain rounds away less
/// than a millionth of a pack over a whole research.
const DURABILITY_ONE: u64 = 1 << 48;
/// A pack with less than this left is used up (absorbs the rounding above).
const DURABILITY_EPSILON: u64 = 1 << 24;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LabState {
    pub energy: EnergyState,
    /// One slot per accepted science pack, in the prototype's `inputs` order.
    pub input: Inventory,
    /// Durability left in the opened pack of each slot, in 2^-48 packs.
    pub opened: Vec<u64>,
    pub working: bool,
}

impl LabState {
    pub fn new(proto: &EntityProto) -> Self {
        let EntityData::Lab { inputs, energy_source, .. } = &proto.data else { unreachable!() };
        LabState {
            energy: EnergyState::for_source(energy_source),
            input: Inventory::new(inputs.len() as u32),
            opened: vec![0; inputs.len()],
            working: false,
        }
    }

    /// Durability of the opened pack in each slot, 0..=1, for display.
    pub fn opened_fraction(&self, slot: usize) -> Fixed {
        Fixed::from_raw((self.opened.get(slot).copied().unwrap_or(0) >> (48 - Fixed::FRAC_BITS)) as i64)
    }

    /// How many of `item` fit; automated inserters stop at a small buffer.
    pub fn room_for(&self, db: &PrototypeDb, proto: &EntityProto, item: ItemId, automated: bool) -> u32 {
        let EntityData::Lab { inputs, .. } = &proto.data else { return 0 };
        let Some(slot) = inputs.iter().position(|i| *i == item) else { return 0 };
        let have = self.input.slot(slot).map_or(0, |s| s.count);
        let limit = if automated { LAB_AUTOMATED_LIMIT } else { db.item(item).stack_size };
        limit.saturating_sub(have)
    }

    pub fn insert(&mut self, db: &PrototypeDb, proto: &EntityProto, item: ItemId, count: u32, automated: bool) -> u32 {
        let EntityData::Lab { inputs, .. } = &proto.data else { return 0 };
        let n = count.min(self.room_for(db, proto, item, automated));
        if n == 0 {
            return 0;
        }
        let slot = inputs.iter().position(|i| *i == item).unwrap();
        let have = self.input.slot(slot).map_or(0, |s| s.count);
        self.input.set_slot(slot, Some(ItemStack::new(item, have + n)));
        n
    }
}

/// Science packs inserters keep in each lab slot. Not confirmed against the game yet.
pub const LAB_AUTOMATED_LIMIT: u32 = 2;

pub(crate) fn update_lab(sim: &mut Simulation, id: EntityId) {
    let db = sim.db.clone();
    let e = sim.entities.get_mut(&id).unwrap();
    let proto = db.entity(e.proto);
    let EntityState::Lab(lab) = &mut e.state else { return };
    let EntityData::Lab { inputs, researching_speed, energy_usage, energy_source } = &proto.data else { return };

    if let EnergySource::Electric { drain, .. } = energy_source {
        lab.energy.draw(&db, energy_source, *drain);
    }
    lab.working = false;
    let r = &sim.research;
    let Some(t) = r.current() else { return };
    let Some(unit) = &db.technology(t).unit else { return };
    // The lab must take every pack the research needs.
    let slots: Option<Vec<(usize, u32)>> = unit
        .ingredients
        .iter()
        .map(|(item, amount)| inputs.iter().position(|i| i == item).map(|s| (s, *amount)))
        .collect();
    let Some(slots) = slots else { return };
    if slots.iter().any(|(s, _)| lab.opened[*s] < DURABILITY_EPSILON && lab.input.slot(*s).is_none()) {
        return;
    }

    let got = lab.energy.draw(&db, energy_source, *energy_usage);
    let frac = if !energy_usage.is_positive() || got >= *energy_usage { Fixed::ONE } else { got / *energy_usage };
    let speed = *researching_speed * (Fixed::ONE + r.modifier("laboratory-speed", None)) * frac;
    if !speed.is_positive() {
        return;
    }
    lab.working = true;
    let time = unit.time_ticks.max(1) as u128;
    for (s, amount) in slots {
        let drain = (amount as u128 * speed.raw() as u128 * (DURABILITY_ONE as u128 >> Fixed::FRAC_BITS) / time) as u64;
        let mut left = drain;
        while left > 0 {
            if lab.opened[s] < DURABILITY_EPSILON {
                let Some(stack) = lab.input.slot(s) else { break };
                lab.input.set_slot(s, Some(ItemStack::new(stack.item, stack.count - 1)));
                let durability = db.item(stack.item).durability.unwrap_or(Fixed::ONE);
                lab.opened[s] += (durability.raw() as u64) << (48 - Fixed::FRAC_BITS);
            }
            let take = left.min(lab.opened[s]);
            lab.opened[s] -= take;
            left -= take;
        }
        if lab.opened[s] < DURABILITY_EPSILON {
            lab.opened[s] = 0;
        }
    }
    let productivity = Fixed::ONE + sim.research.modifier("laboratory-productivity", None);
    let work = (speed * productivity).raw() as u128;
    sim.add_research_work(work);
}

#[cfg(test)]
mod tests {
    use crate::proto::CountFormula;

    #[test]
    fn count_formulas() {
        let f = CountFormula::parse("2^(L-7)*1000").unwrap();
        assert_eq!(f.eval(7), 1000);
        assert_eq!(f.eval(9), 4000);
        assert_eq!(CountFormula::parse("1000+3^(L-1)*1000").unwrap().eval(2), 4000);
        assert_eq!(CountFormula::parse("2500*(L - 3)").unwrap().eval(4), 2500);
        assert_eq!(CountFormula::parse("2^3^2").unwrap().eval(1), 512);
    }
}
