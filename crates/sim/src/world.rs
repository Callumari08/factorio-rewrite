//! Simulation state, entity storage, building and the fixed tick.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::belt::{BeltKind, BeltState, BeltSystem};
use crate::energy::{Burner, EnergyState};
use crate::fixed::Fixed;
use crate::input::{InputAction, PlayerInput};
use crate::inventory::{Inventory, ItemStack};
use crate::machines::{CrafterState, DrillState, InserterState};
use crate::map::{Area, ChunkPosition, Direction, MapPosition, SUBTILES_PER_TILE, TilePosition};
use crate::player::Player;
use crate::power::{FluidEntity, PowerSystem};
use crate::proto::{EntityData, EntityProto, EntityProtoId, ItemId, PrototypeDb};
use crate::rng::DetRng;
use crate::surface::{MapGenSettings, Surface};

pub type Tick = u64;

/// Stable entity handle. Ids are allocated monotonically and never reused, so iterating
/// entities by id is a stable, platform-independent order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Entity {
    pub proto: EntityProtoId,
    pub position: MapPosition,
    pub direction: Direction,
    pub state: EntityState,
}

/// Per-type runtime state. Belt contents live in [`BeltSystem`] and fluid/electric
/// networks in [`PowerSystem`]; their entities only carry a marker here.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum EntityState {
    #[default]
    None,
    Container(Inventory),
    Drill(DrillState),
    Crafter(CrafterState),
    Inserter(InserterState),
    Belt,
    Pole,
    Fluid(FluidEntity),
}

/// Why something could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    NotPlaceable,
    MissingItem,
    OutOfReach,
    Ungenerated,
    TileCollision,
    EntityCollision,
    CharacterCollision,
    NoResources,
}

/// Who is inserting an item into an entity. Automated sources respect Factorio's
/// automatic insertion limits; the player does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertSource {
    Player,
    Automated,
}

/// How many tiles around each character are kept generated.
const GENERATE_RADIUS_CHUNKS: i32 = 3;

/// The complete deterministic game state.
#[derive(Clone, Debug)]
pub struct Simulation {
    pub(crate) db: Arc<PrototypeDb>,
    pub(crate) tick: Tick,
    pub(crate) rng: DetRng,
    next_entity_id: u64,
    pub surface: Surface,
    pub(crate) entities: BTreeMap<EntityId, Entity>,
    /// Footprint tile -> entity occupying it.
    tile_index: BTreeMap<TilePosition, EntityId>,
    pub belts: BeltSystem,
    pub power: PowerSystem,
    pub(crate) ground_items: BTreeMap<MapPosition, ItemStack>,
    pub(crate) players: BTreeMap<u16, Player>,
}

impl Simulation {
    pub fn new(prototypes: Arc<PrototypeDb>, mapgen: MapGenSettings) -> Self {
        let mut sim = Simulation {
            rng: DetRng::new(mapgen.seed),
            db: prototypes,
            tick: 0,
            next_entity_id: 1,
            surface: Surface::new(mapgen),
            entities: BTreeMap::new(),
            tile_index: BTreeMap::new(),
            belts: BeltSystem::default(),
            power: PowerSystem::default(),
            ground_items: BTreeMap::new(),
            players: BTreeMap::new(),
        };
        sim.generate_around(MapPosition::default());
        sim
    }

    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn prototypes(&self) -> &PrototypeDb {
        &self.db
    }

    pub fn prototypes_arc(&self) -> Arc<PrototypeDb> {
        self.db.clone()
    }

    pub fn entities(&self) -> impl Iterator<Item = (EntityId, &Entity)> {
        self.entities.iter().map(|(id, e)| (*id, e))
    }

    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn players(&self) -> impl Iterator<Item = (u16, &Player)> {
        self.players.iter().map(|(id, p)| (*id, p))
    }

    pub fn player(&self, id: u16) -> Option<&Player> {
        self.players.get(&id)
    }

    pub fn ground_items(&self) -> impl Iterator<Item = (MapPosition, ItemStack)> + '_ {
        self.ground_items.iter().map(|(p, s)| (*p, *s))
    }

    pub fn rng(&mut self) -> &mut DetRng {
        &mut self.rng
    }

    /// Advances the world by exactly one tick.
    ///
    /// `inputs` are sorted before being applied, so peers that receive the same set of
    /// inputs in a different network order still compute the same state.
    pub fn step(&mut self, inputs: &[PlayerInput]) {
        let mut inputs = inputs.to_vec();
        inputs.sort();
        for input in &inputs {
            self.apply_input(input);
        }

        let positions: Vec<MapPosition> =
            self.players.values().filter_map(|p| p.character.as_ref()).map(|c| c.position()).collect();
        for p in positions {
            self.generate_around(p);
        }

        let ids: Vec<u16> = self.players.keys().copied().collect();
        for id in ids {
            crate::player::update(self, id);
        }

        crate::power::update(self);

        let ids: Vec<EntityId> = self.entities.keys().copied().collect();
        for id in ids {
            crate::machines::update_entity(self, id);
        }

        self.belts.update();

        self.tick += 1;
    }

    fn generate_around(&mut self, p: MapPosition) {
        let c = p.tile().chunk();
        for y in c.y - GENERATE_RADIUS_CHUNKS..=c.y + GENERATE_RADIUS_CHUNKS {
            for x in c.x - GENERATE_RADIUS_CHUNKS..=c.x + GENERATE_RADIUS_CHUNKS {
                self.surface.ensure_chunk(ChunkPosition { x, y });
            }
        }
    }

    fn apply_input(&mut self, input: &PlayerInput) {
        let player = input.player;
        if let InputAction::JoinGame = input.action {
            self.join(player);
            return;
        }
        if let InputAction::CheatPlaceEntity { entity, position, direction } = input.action {
            let _ = self.place_entity(entity, position, direction);
            return;
        }
        if let InputAction::CheatSetRecipe { position, recipe } = input.action {
            if let Some(id) = self.entity_at(position)
                && let Some(e) = self.entities.get_mut(&id)
                && let EntityState::Crafter(c) = &mut e.state
            {
                let db = self.db.clone();
                c.set_recipe(&db, db.entity(e.proto), Some(recipe));
            }
            return;
        }
        if let InputAction::CheatInsert { position, item, count } = input.action {
            if let Some(id) = self.entity_at(position) {
                self.insert_into_entity(id, item, count, InsertSource::Player);
            }
            return;
        }
        crate::player::apply_input(self, player, &input.action);
    }

    fn join(&mut self, player: u16) {
        if self.players.get(&player).is_some_and(|p| p.character.is_some()) {
            return;
        }
        let Some(proto) = self.db.special.character else { return };
        // Spawn at the origin, or the nearest walkable tile centre spiralling outwards.
        let mut spot = MapPosition::default();
        'search: for r in 0..32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    let p = MapPosition::tile_center(TilePosition::new(dx, dy));
                    if crate::player::character_fits(self, proto, p) {
                        spot = p;
                        break 'search;
                    }
                }
            }
        }
        let character = crate::player::Character::new(&self.db, proto, spot);
        self.players.entry(player).or_default().character = Some(character);
    }

    // ----- spatial queries -----

    /// The entity whose footprint covers the tile containing `p`.
    pub fn entity_at(&self, p: MapPosition) -> Option<EntityId> {
        self.tile_index.get(&p.tile()).copied()
    }

    pub fn entity_at_tile(&self, t: TilePosition) -> Option<EntityId> {
        self.tile_index.get(&t).copied()
    }

    pub fn footprint(proto: &EntityProto, position: MapPosition, direction: Direction) -> Area {
        let (w, h) = proto.tile_size(direction);
        let half_w = w * SUBTILES_PER_TILE / 2;
        let half_h = h * SUBTILES_PER_TILE / 2;
        Area {
            left_top: MapPosition::new(position.x - half_w, position.y - half_h),
            right_bottom: MapPosition::new(position.x + half_w, position.y + half_h),
        }
    }

    /// Entities whose footprint touches the area.
    pub fn entities_in(&self, area: Area) -> Vec<EntityId> {
        let mut out: Vec<EntityId> = area.tiles().filter_map(|t| self.tile_index.get(&t).copied()).collect();
        out.sort();
        out.dedup();
        out
    }

    // ----- building -----

    pub fn can_place(
        &self,
        proto_id: EntityProtoId,
        position: MapPosition,
        direction: Direction,
    ) -> Result<(), BuildError> {
        let proto = self.db.entity(proto_id);
        if matches!(proto.data, EntityData::Resource { .. } | EntityData::Character { .. }) {
            return Err(BuildError::NotPlaceable);
        }
        let footprint = Self::footprint(proto, position, direction);
        for t in footprint.tiles() {
            if !self.surface.is_generated(t.chunk()) {
                return Err(BuildError::Ungenerated);
            }
        }
        if !crate::power::buildable_on_tiles(self, proto, position, direction) {
            return Err(BuildError::TileCollision);
        }
        let area = proto.rotated_box(direction).at(position);
        // Entities never share footprint tiles in this implementation.
        if !self.entities_in(footprint).is_empty() {
            return Err(BuildError::EntityCollision);
        }
        for p in self.players.values() {
            if let Some(c) = &p.character {
                let cp = self.db.entity(c.proto);
                if proto.collision_mask.collides(&cp.collision_mask) && c.collision_area(&self.db).overlaps(&area) {
                    return Err(BuildError::CharacterCollision);
                }
            }
        }
        if let EntityData::MiningDrill { .. } = proto.data
            && crate::machines::drill_resources(self, proto, position, direction).is_empty()
        {
            return Err(BuildError::NoResources);
        }
        Ok(())
    }

    /// Places an entity if the location allows it. Does not take items or check reach.
    pub fn place_entity(
        &mut self,
        proto_id: EntityProtoId,
        position: MapPosition,
        direction: Direction,
    ) -> Result<EntityId, BuildError> {
        let proto = self.db.entity(proto_id).clone();
        let direction = if proto.rotatable || matches!(proto.data, EntityData::TransportBelt { .. }) {
            Direction(direction.0 / 4 * 4)
        } else {
            Direction::NORTH
        };
        let position = proto.snap_position(position, direction);
        self.can_place(proto_id, position, direction)?;

        let id = EntityId(self.next_entity_id);
        self.next_entity_id += 1;
        let state = self.initial_state(id, &proto, position, direction);
        for t in Self::footprint(&proto, position, direction).tiles() {
            self.tile_index.insert(t, id);
        }
        self.entities.insert(id, Entity { proto: proto_id, position, direction, state });
        self.power.mark_dirty();
        Ok(id)
    }

    fn initial_state(
        &mut self,
        id: EntityId,
        proto: &EntityProto,
        position: MapPosition,
        direction: Direction,
    ) -> EntityState {
        match &proto.data {
            EntityData::Container { inventory_size } => EntityState::Container(Inventory::new(*inventory_size)),
            EntityData::MiningDrill { energy_source, .. } => EntityState::Drill(DrillState::new(energy_source)),
            EntityData::CraftingMachine { .. } => EntityState::Crafter(CrafterState::new(proto)),
            EntityData::Inserter { energy_source, .. } => {
                EntityState::Inserter(InserterState::new(proto, energy_source))
            }
            EntityData::TransportBelt { speed } => {
                self.belts.add(id, BeltState::new(BeltKind::Belt, position, direction, *speed));
                EntityState::Belt
            }
            EntityData::UndergroundBelt { speed, max_distance } => {
                // Becomes an exit if an unpaired entrance facing the same way is behind it.
                let mut kind = BeltKind::UndergroundInput;
                let mut t = position.tile();
                for _ in 0..*max_distance {
                    t = t.step(direction.opposite());
                    if let Some(other) = self.belts.at_tile(t)
                        && let Some(b) = self.belts.get(other)
                        && b.direction == direction
                    {
                        if b.kind == BeltKind::UndergroundInput {
                            kind = BeltKind::UndergroundOutput;
                        }
                        if matches!(b.kind, BeltKind::UndergroundInput | BeltKind::UndergroundOutput) {
                            break;
                        }
                    }
                }
                let mut state = BeltState::new(kind, position, direction, *speed);
                state.max_distance = *max_distance;
                self.belts.add(id, state);
                EntityState::Belt
            }
            EntityData::Splitter { speed } => {
                self.belts.add(id, BeltState::new(BeltKind::Splitter, position, direction, *speed));
                EntityState::Belt
            }
            EntityData::ElectricPole { .. } => EntityState::Pole,
            EntityData::OffshorePump { .. }
            | EntityData::Boiler { .. }
            | EntityData::Generator { .. }
            | EntityData::Pipe { .. } => EntityState::Fluid(FluidEntity::new(proto)),
            _ => EntityState::None,
        }
    }

    /// Removes an entity and returns everything it contained (not the entity item itself).
    pub fn remove_entity(&mut self, id: EntityId) -> Vec<ItemStack> {
        let Some(e) = self.entities.remove(&id) else { return Vec::new() };
        let proto = self.db.entity(e.proto);
        for t in Self::footprint(proto, e.position, e.direction).tiles() {
            if self.tile_index.get(&t) == Some(&id) {
                self.tile_index.remove(&t);
            }
        }
        self.power.mark_dirty();
        let mut out: Vec<ItemStack> = Vec::new();
        fn add_inv(out: &mut Vec<ItemStack>) -> impl FnMut(&mut Inventory) + '_ {
            move |inv: &mut Inventory| out.extend(inv.take_all())
        }
        match e.state {
            EntityState::Container(mut inv) => add_inv(&mut out)(&mut inv),
            EntityState::Drill(mut d) => {
                d.energy.take_contents(&mut add_inv(&mut out));
                out.extend(d.output);
            }
            EntityState::Crafter(mut c) => {
                add_inv(&mut out)(&mut c.input);
                add_inv(&mut out)(&mut c.output);
                c.energy.take_contents(&mut add_inv(&mut out));
                if c.crafting
                    && let Some(r) = c.recipe
                {
                    // Ingredients of an unfinished craft are returned, as in Factorio.
                    for ing in &self.db.recipe(r).ingredients {
                        if let crate::proto::ItemOrFluid::Item(i) = ing.what {
                            out.push(ItemStack::new(i, ing.amount.floor_int() as u32));
                        }
                    }
                }
            }
            EntityState::Inserter(mut i) => {
                out.extend(i.hand.take());
                i.energy.take_contents(&mut add_inv(&mut out));
            }
            EntityState::Belt => {
                if let Some(mut b) = self.belts.remove(id) {
                    for item in b.take_all() {
                        out.push(ItemStack::new(item, 1));
                    }
                }
            }
            EntityState::Fluid(mut f) => f.energy.take_contents(&mut add_inv(&mut out)),
            _ => {}
        }
        out
    }

    pub fn rotate_entity(&mut self, id: EntityId, reverse: bool) -> bool {
        let Some(e) = self.entities.get(&id) else { return false };
        let proto = self.db.entity(e.proto);
        if !proto.rotatable {
            return false;
        }
        let (w, h) = (proto.tile_width, proto.tile_height);
        let new_dir = if reverse { e.direction.rotate_ccw() } else { e.direction.rotate_cw() };
        // Non-square entities would change footprint; only allow if the area is free.
        if w != h {
            return false;
        }
        let e = self.entities.get_mut(&id).unwrap();
        e.direction = new_dir;
        if let EntityState::Belt = e.state {
            self.belts.set_direction(id, new_dir);
        }
        if let EntityState::Inserter(i) = &mut e.state {
            i.reset_after_rotation();
        }
        self.power.mark_dirty();
        true
    }

    // ----- ground items -----

    /// Puts a stack on the ground near `at`, at the nearest free quarter-tile spot.
    pub fn spill(&mut self, at: MapPosition, stack: ItemStack) {
        if stack.count == 0 {
            return;
        }
        let q = SUBTILES_PER_TILE / 4;
        let base = MapPosition::new(at.x.div_euclid(q) * q + q / 2, at.y.div_euclid(q) * q + q / 2);
        for r in 0..64i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let p = base.offset(dx * q, dy * q);
                    if self.ground_items.contains_key(&p) || !self.surface.is_generated(p.tile().chunk()) {
                        continue;
                    }
                    if self.tile_blocks_items(p.tile()) {
                        continue;
                    }
                    self.ground_items.insert(p, stack);
                    return;
                }
            }
        }
    }

    fn tile_blocks_items(&self, t: TilePosition) -> bool {
        // Items cannot be placed on water: tiles colliding with the item-entity mask.
        let Some(tile) = self.surface.tile(t) else { return true };
        let mask = &self.db.tile(tile).collision_mask;
        let item_layer = self.db.collision_layers.iter().position(|l| l == "item").map(|i| 1u64 << i).unwrap_or(0);
        mask.layers & item_layer != 0
    }

    /// Drops a single item exactly at `p` if nothing is lying there (used by drills and
    /// inserters dropping onto the ground).
    pub fn drop_on_ground(&mut self, p: MapPosition, item: ItemId) -> bool {
        let r = SUBTILES_PER_TILE * 28 / 100;
        let blocked = self
            .ground_items
            .range(MapPosition::new(p.x - r, i32::MIN)..=MapPosition::new(p.x + r, i32::MAX))
            .any(|(gp, _)| (gp.x - p.x).abs() < r && (gp.y - p.y).abs() < r);
        if blocked || self.tile_blocks_items(p.tile()) {
            return false;
        }
        self.ground_items.insert(p, ItemStack::new(item, 1));
        true
    }

    /// Takes one item lying within a tile-sized box around `p`.
    pub fn take_from_ground(&mut self, p: MapPosition, wants: &dyn Fn(ItemId) -> bool) -> Option<ItemId> {
        let r = SUBTILES_PER_TILE / 2;
        let key = self
            .ground_items
            .range(MapPosition::new(p.x - r, i32::MIN)..=MapPosition::new(p.x + r, i32::MAX))
            .find(|(gp, s)| (gp.y - p.y).abs() <= r && (gp.x - p.x).abs() <= r && wants(s.item))
            .map(|(gp, _)| *gp)?;
        let stack = self.ground_items.get_mut(&key).unwrap();
        stack.count -= 1;
        let item = stack.item;
        if stack.count == 0 {
            self.ground_items.remove(&key);
        }
        Some(item)
    }

    // ----- moving items in and out of entities -----

    /// Inserts up to `count` of `item` into the entity; returns how many went in.
    pub fn insert_into_entity(&mut self, id: EntityId, item: ItemId, count: u32, source: InsertSource) -> u32 {
        let db = self.db.clone();
        let Some(e) = self.entities.get_mut(&id) else { return 0 };
        let proto = db.entity(e.proto);
        let fuel_limit = |b: &Burner| match source {
            InsertSource::Player => u32::MAX,
            // Inserters stop topping up fuel once a few items are in (Factorio's automatic
            // insertion limit for fuel).
            InsertSource::Automated => 5u32.saturating_sub(b.fuel.count(item)),
        };
        let try_fuel = |energy: &mut EnergyState, count: u32| -> u32 {
            let Some(src) = proto.energy_source() else { return 0 };
            match energy.burner_mut() {
                Some(b) if Burner::accepts(&db, src, item) => {
                    let n = count.min(fuel_limit(b));
                    b.fuel.insert(&db, item, n)
                }
                _ => 0,
            }
        };
        match &mut e.state {
            EntityState::Container(inv) => inv.insert(&db, item, count),
            EntityState::Drill(d) => try_fuel(&mut d.energy, count),
            EntityState::Inserter(i) => try_fuel(&mut i.energy, count),
            EntityState::Fluid(f) => try_fuel(&mut f.energy, count),
            EntityState::Crafter(c) => {
                let fuel = try_fuel(&mut c.energy, count);
                if fuel > 0 {
                    return fuel;
                }
                c.insert_ingredient(&db, proto, item, count, source)
            }
            _ => 0,
        }
    }

    /// Whether an automated source could insert `item` right now.
    pub fn entity_wants(&self, id: EntityId, item: ItemId) -> bool {
        let Some(e) = self.entities.get(&id) else { return false };
        let db = &self.db;
        let proto = db.entity(e.proto);
        let fuel_ok = |energy: &EnergyState| match (energy.burner(), proto.energy_source()) {
            (Some(b), Some(src)) => {
                Burner::accepts(db, src, item) && b.fuel.count(item) < 5 && b.fuel.space_for(db, item) > 0
            }
            _ => false,
        };
        match &e.state {
            EntityState::Container(inv) => inv.space_for(db, item) > 0,
            EntityState::Drill(d) => fuel_ok(&d.energy),
            EntityState::Inserter(i) => fuel_ok(&i.energy),
            EntityState::Fluid(f) => fuel_ok(&f.energy),
            EntityState::Crafter(c) => {
                fuel_ok(&c.energy) || c.ingredient_room(db, proto, item, InsertSource::Automated) > 0
            }
            _ => false,
        }
    }

    /// Items an inserter could take from this entity.
    pub fn items_for_inserter(&self, id: EntityId) -> Vec<ItemId> {
        let Some(e) = self.entities.get(&id) else { return Vec::new() };
        let items = |inv: &Inventory| inv.slots().iter().flatten().map(|s| s.item).collect::<Vec<_>>();
        match &e.state {
            EntityState::Container(inv) => items(inv),
            EntityState::Crafter(c) => {
                let mut v = items(&c.output);
                if let Some(b) = c.energy.burner() {
                    v.extend(items(&b.burnt));
                }
                v
            }
            EntityState::Drill(d) => d.energy.burner().map(|b| items(&b.burnt)).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Items lying within a tile-sized box around `p`.
    pub fn ground_items_near(&self, p: MapPosition) -> Vec<ItemId> {
        let r = SUBTILES_PER_TILE / 2;
        self.ground_items
            .range(MapPosition::new(p.x - r, i32::MIN)..=MapPosition::new(p.x + r, i32::MAX))
            .filter(|(gp, _)| (gp.y - p.y).abs() <= r && (gp.x - p.x).abs() <= r)
            .map(|(_, s)| s.item)
            .collect()
    }

    /// Takes one item an inserter may pick up from the entity (containers: anything;
    /// machines: their output).
    pub fn take_for_inserter(&mut self, id: EntityId, wants: &dyn Fn(ItemId) -> bool) -> Option<ItemId> {
        let e = self.entities.get_mut(&id)?;
        let take = |inv: &mut Inventory| {
            let item = inv.slots().iter().flatten().map(|s| s.item).find(|i| wants(*i))?;
            inv.remove(item, 1);
            Some(item)
        };
        match &mut e.state {
            EntityState::Container(inv) => take(inv),
            EntityState::Crafter(c) => {
                take(&mut c.output).or_else(|| c.energy.burner_mut().and_then(|b| take(&mut b.burnt)))
            }
            EntityState::Drill(d) => d.energy.burner_mut().and_then(|b| take(&mut b.burnt)),
            _ => None,
        }
    }

    // ----- determinism -----

    /// A hash of the full game state, for desync detection between lockstep peers.
    /// FNV-1a over a fixed field order, so it is stable across platforms and builds.
    pub fn checksum(&self) -> u64 {
        let mut h = Fnv::new();
        self.tick.hash(&mut h);
        self.rng.state().hash(&mut h);
        self.next_entity_id.hash(&mut h);
        self.entities.hash(&mut h);
        for (id, b) in &self.belts.belts {
            id.hash(&mut h);
            b.hash(&mut h);
        }
        self.ground_items.hash(&mut h);
        self.players.hash(&mut h);
        for (pos, chunk) in self.surface.chunks() {
            pos.hash(&mut h);
            chunk.hash(&mut h);
        }
        h.finish()
    }

    /// Integer hit test for the character's position etc.
    pub fn fixed_to_map(x: Fixed, y: Fixed) -> MapPosition {
        MapPosition::new((x.raw() >> (Fixed::FRAC_BITS - 8)) as i32, (y.raw() >> (Fixed::FRAC_BITS - 8)) as i32)
    }
}

impl EnergyState {
    pub(crate) fn take_contents(&mut self, f: &mut impl FnMut(&mut Inventory)) {
        if let EnergyState::Burner(b) = self {
            f(&mut b.fuel);
            f(&mut b.burnt);
        }
    }
}

/// FNV-1a. `usize` is always hashed as 8 bytes so 32- and 64-bit peers agree.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn write_usize(&mut self, i: usize) {
        self.write(&(i as u64).to_le_bytes());
    }
    fn write_isize(&mut self, i: isize) {
        self.write(&(i as i64).to_le_bytes());
    }
    fn finish(&self) -> u64 {
        self.0
    }
}
