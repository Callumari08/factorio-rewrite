//! Typed, simulation-ready prototypes.
//!
//! Produced from Factorio's Lua data stage by the `factorio-data` crate, with every float
//! converted to fixed point at load time. Prototypes are stored in vectors sorted by name
//! and referenced by small integer ids, so ids are identical on every machine that loads
//! the same mod set. Nothing here names a base-game prototype: everything comes from data.

use std::collections::BTreeMap;

use crate::fixed::Fixed;
use crate::map::{BoundingBox, Direction, MapPosition, SUBTILES_PER_TILE, TilePosition};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u16);
        impl $name {
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

id_type!(ItemId);
id_type!(FluidId);
id_type!(RecipeId);
id_type!(EntityProtoId);
id_type!(TileId);
id_type!(TechId);

/// Energy in joules. Power values are stored as joules per tick.
pub type Energy = Fixed;

/// A set of collision layers (Factorio 2.0 `CollisionMask.layers`), one bit per layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CollisionMask {
    pub layers: u64,
    pub not_colliding_with_itself: bool,
    pub colliding_with_tiles_only: bool,
}

impl CollisionMask {
    pub fn collides(&self, other: &CollisionMask) -> bool {
        if self.not_colliding_with_itself && other.not_colliding_with_itself && self.layers == other.layers {
            return false;
        }
        self.layers & other.layers != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemProto {
    pub name: String,
    /// Prototype `type`, e.g. `item`, `tool`, `ammo`.
    pub kind: String,
    pub stack_size: u32,
    pub place_result: Option<EntityProtoId>,
    pub fuel: Option<Fuel>,
    /// Position in Factorio's item ordering (group, subgroup, order, name), used when
    /// sorting inventories.
    pub sort_index: u32,
    /// Durability of `tool` items (science packs); a lab uses it up while researching.
    pub durability: Option<Fixed>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fuel {
    pub category: String,
    pub value: Energy,
    pub burnt_result: Option<ItemId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FluidProto {
    pub name: String,
    pub default_temperature: Fixed,
    /// Joules per unit per degree.
    pub heat_capacity: Energy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemOrFluid {
    Item(ItemId),
    Fluid(FluidId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ingredient {
    pub what: ItemOrFluid,
    pub amount: Fixed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Product {
    pub what: ItemOrFluid,
    /// Minimum and maximum amount; equal for fixed amounts.
    pub amount_min: Fixed,
    pub amount_max: Fixed,
    pub probability: Fixed,
    /// Fluid temperature, if specified.
    pub temperature: Option<Fixed>,
}

impl Product {
    /// The guaranteed integer amount for fixed-amount, probability-1 products.
    pub fn fixed_count(&self) -> Option<u32> {
        (self.amount_min == self.amount_max && self.probability == Fixed::ONE && self.amount_min.frac_is_zero())
            .then(|| self.amount_min.floor_int() as u32)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeProto {
    pub name: String,
    pub category: String,
    /// Crafting time in seconds at crafting speed 1.
    pub energy_required: Fixed,
    /// The same in ticks, converted from the exact decimal at load time (3.2 s = 192).
    pub energy_required_ticks: Fixed,
    pub ingredients: Vec<Ingredient>,
    pub results: Vec<Product>,
    pub enabled: bool,
    /// The item this recipe is "for" when choosing a recipe to craft an intermediate.
    pub main_product: Option<ItemOrFluid>,
    pub allow_as_intermediate: bool,
    pub hidden: bool,
}

impl RecipeProto {
    /// Crafting time in ticks at crafting speed 1.
    pub fn ticks(&self) -> Fixed {
        self.energy_required_ticks
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileProto {
    pub name: String,
    pub collision_mask: CollisionMask,
    pub walking_speed_modifier: Fixed,
    pub map_color: [u8; 3],
    /// Fluid that offshore pumps extract from this tile.
    pub fluid: Option<FluidId>,
    pub layer: i32,
    pub autoplace: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Minable {
    pub mining_time: Fixed,
    /// Mining time in ticks at mining speed 1, converted exactly at load time.
    pub mining_ticks: Fixed,
    pub results: Vec<Product>,
    pub required_fluid: Option<FluidId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnergySource {
    Burner {
        effectivity: Fixed,
        fuel_inventory_size: u32,
        fuel_categories: Vec<String>,
    },
    Electric {
        drain: Energy,
        buffer_capacity: Option<Energy>,
        output: bool,
    },
    Void,
    /// Heat and fluid energy sources are not simulated yet.
    Unsupported,
}

impl EnergySource {
    pub fn is_burner(&self) -> bool {
        matches!(self, EnergySource::Burner { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowDirection {
    Input,
    Output,
    InputOutput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeConnection {
    /// Position of the connecting tile relative to the entity centre, entity facing north,
    /// in 1/256 tiles.
    pub position: [i32; 2],
    pub direction: Direction,
    pub flow: FlowDirection,
    /// For `pipe-to-ground` style connections.
    pub underground_max_distance: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FluidBoxProto {
    pub volume: Fixed,
    pub filter: Option<FluidId>,
    pub connections: Vec<PipeConnection>,
    pub minimum_temperature: Option<Fixed>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntityData {
    Resource {
        infinite: bool,
        category: String,
        /// Amount thresholds used to choose a sprite stage.
        stage_counts: Vec<u32>,
        /// Whether map generation should place this resource (has `autoplace`).
        autoplace: bool,
    },
    Character {
        running_speed: Fixed,
        mining_speed: Fixed,
        inventory_size: u32,
        build_distance: Fixed,
        reach_distance: Fixed,
        reach_resource_distance: Fixed,
        item_pickup_distance: Fixed,
        loot_pickup_distance: Fixed,
        mining_categories: Vec<String>,
        crafting_categories: Vec<String>,
    },
    Container {
        inventory_size: u32,
    },
    MiningDrill {
        mining_speed: Fixed,
        energy_usage: Energy,
        energy_source: EnergySource,
        /// Half-size of the mining area in 1/256 tiles.
        radius: i32,
        /// Output offset from the drill centre for a north-facing drill, 1/256 tiles.
        output_vector: [i32; 2],
        resource_categories: Vec<String>,
    },
    CraftingMachine {
        furnace: bool,
        crafting_speed: Fixed,
        crafting_categories: Vec<String>,
        energy_usage: Energy,
        energy_source: EnergySource,
        source_inventory_size: Option<u32>,
        result_inventory_size: Option<u32>,
        fixed_recipe: Option<RecipeId>,
        fluid_boxes: Vec<FluidBoxProto>,
    },
    Inserter {
        /// Turns per tick.
        rotation_speed: Fixed,
        /// Tiles per tick.
        extension_speed: Fixed,
        /// Positions relative to the inserter centre for a north-facing inserter, in tiles.
        pickup_position: [Fixed; 2],
        insert_position: [Fixed; 2],
        energy_per_movement: Energy,
        energy_per_rotation: Energy,
        energy_source: EnergySource,
        /// Uses the bulk inserter capacity bonus instead of the inserter stack size bonus.
        bulk: bool,
        /// Hand size above 1 without research.
        stack_size_bonus: u32,
        /// Whether research bonuses raise the hand size.
        uses_stack_size_bonus: bool,
        /// How many item filters the inserter can have.
        filter_count: u8,
        /// Waits for a full hand before leaving the pickup position.
        wait_for_full_hand: bool,
    },
    TransportBelt {
        /// Belt positions (1/256 tile) per tick.
        speed: i32,
    },
    UndergroundBelt {
        speed: i32,
        max_distance: u32,
    },
    Splitter {
        speed: i32,
    },
    ElectricPole {
        supply_area_distance: Fixed,
        maximum_wire_distance: Fixed,
    },
    OffshorePump {
        /// Fluid units per tick.
        pumping_speed: Fixed,
        /// Where the water is taken from, relative to the pump facing north, 1/256 tiles.
        fluid_source_offset: [i32; 2],
        fluid_box: FluidBoxProto,
    },
    Boiler {
        energy_consumption: Energy,
        energy_source: EnergySource,
        target_temperature: Fixed,
        fluid_box: FluidBoxProto,
        output_fluid_box: FluidBoxProto,
    },
    Generator {
        fluid_usage_per_tick: Fixed,
        maximum_temperature: Fixed,
        effectivity: Fixed,
        fluid_box: FluidBoxProto,
    },
    Pipe {
        fluid_box: FluidBoxProto,
    },
    Lab {
        /// Science packs the lab accepts, one input slot each, in prototype order.
        inputs: Vec<ItemId>,
        researching_speed: Fixed,
        energy_usage: Energy,
        energy_source: EnergySource,
    },
    /// Cliffs: one entity per grid cell, in one of the orientations (sorted by name, e.g.
    /// `west_to_east`), each with its collision box (axis-aligned around a rotated box).
    Cliff {
        orientations: Vec<CliffOrientation>,
        /// Grid cell size and offset in 1/256 tiles.
        grid_size: [i32; 2],
        grid_offset: [i32; 2],
    },
    /// Anything not simulated yet; it can still be placed and drawn.
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliffOrientation {
    pub name: String,
    pub collision_box: BoundingBox,
    pub variations: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityProto {
    pub name: String,
    /// Prototype `type`, e.g. `transport-belt`, `assembling-machine`.
    pub kind: String,
    pub collision_box: BoundingBox,
    pub collision_mask: CollisionMask,
    /// Footprint in tiles when facing north.
    pub tile_width: i32,
    pub tile_height: i32,
    pub minable: Option<Minable>,
    /// Whether the entity can be rotated by the player.
    pub rotatable: bool,
    /// `tile_buildability_rules`; when present they replace the plain tile collision test.
    pub tile_rules: Vec<TileRule>,
    pub data: EntityData,
}

/// One of Factorio's `TileBuildabilityRule`s: every tile in `area` must have one of the
/// `required` layers and none of the `colliding` layers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileRule {
    pub area: BoundingBox,
    pub required: u64,
    pub colliding: u64,
}

impl EntityProto {
    /// Footprint in whole tiles for the given direction.
    pub fn tile_size(&self, direction: Direction) -> (i32, i32) {
        if direction.is_horizontal() {
            (self.tile_height, self.tile_width)
        } else {
            (self.tile_width, self.tile_height)
        }
    }

    /// Snaps a desired centre position to the tile grid the way Factorio does for
    /// building: odd sizes centre on a tile, even sizes on a tile corner.
    pub fn snap_position(&self, at: MapPosition, direction: Direction) -> MapPosition {
        let (w, h) = self.tile_size(direction);
        let snap = |v: i32, size: i32| {
            if size % 2 == 1 {
                v.div_euclid(SUBTILES_PER_TILE) * SUBTILES_PER_TILE + SUBTILES_PER_TILE / 2
            } else {
                (v + SUBTILES_PER_TILE / 2).div_euclid(SUBTILES_PER_TILE) * SUBTILES_PER_TILE
            }
        };
        MapPosition { x: snap(at.x, w), y: snap(at.y, h) }
    }

    /// Centre position for an entity whose top-left tile is `tile`.
    pub fn position_for_tile(&self, tile: TilePosition, direction: Direction) -> MapPosition {
        let (w, h) = self.tile_size(direction);
        MapPosition {
            x: tile.x * SUBTILES_PER_TILE + w * SUBTILES_PER_TILE / 2,
            y: tile.y * SUBTILES_PER_TILE + h * SUBTILES_PER_TILE / 2,
        }
    }

    /// Collision box rotated for `direction` (cardinal directions only).
    pub fn rotated_box(&self, direction: Direction) -> BoundingBox {
        self.collision_box.rotated(direction)
    }

    pub fn energy_source(&self) -> Option<&EnergySource> {
        match &self.data {
            EntityData::MiningDrill { energy_source, .. }
            | EntityData::CraftingMachine { energy_source, .. }
            | EntityData::Inserter { energy_source, .. }
            | EntityData::Boiler { energy_source, .. }
            | EntityData::Lab { energy_source, .. } => Some(energy_source),
            _ => None,
        }
    }
}

/// A technology's research cost: `count` units, each taking `time_ticks` at lab speed 1
/// and one `amount` of each science pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResearchUnit {
    pub count: ResearchCount,
    pub ingredients: Vec<(ItemId, u32)>,
    /// Ticks per unit at researching speed 1.
    pub time_ticks: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResearchCount {
    Fixed(u64),
    /// `count_formula` of leveled technologies, in terms of the level `L`.
    Formula(CountFormula),
}

/// Something that researches a technology without labs (Factorio 2.0 `research_trigger`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResearchTrigger {
    CraftItem {
        item: ItemId,
        count: u32,
    },
    MineEntity {
        entity: EntityProtoId,
    },
    BuildEntity {
        entity: EntityProtoId,
    },
    CraftFluid {
        fluid: FluidId,
        amount: Fixed,
    },
    /// Trigger types that cannot happen yet (rockets, space platforms, spawners).
    Other(String),
}

/// One entry of a technology's `effects`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TechEffect {
    UnlockRecipe(RecipeId),
    GiveItem {
        item: ItemId,
        count: u32,
    },
    /// Every other modifier, keyed by its `type` plus its `ammo_category`, `turret_id` or
    /// similar qualifier, e.g. `("ammo-damage", "bullet")`. Boolean effects have modifier 1.
    Modifier {
        kind: String,
        qualifier: String,
        modifier: Fixed,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechnologyProto {
    pub name: String,
    pub prerequisites: Vec<TechId>,
    pub unit: Option<ResearchUnit>,
    pub trigger: Option<ResearchTrigger>,
    pub effects: Vec<TechEffect>,
    pub enabled: bool,
    pub hidden: bool,
    pub upgrade: bool,
    pub essential: bool,
    /// First level, from a trailing `-N` in the name (Factorio's convention), else 1.
    pub level: u32,
    /// Last level; `None` for `max_level = "infinite"`. Equal to `level` for normal techs.
    pub max_level: Option<u32>,
    pub order: String,
}

/// An integer `count_formula`: numbers, `L`/`l`, `+ - * / ^` and parentheses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CountFormula {
    Num(i128),
    Level,
    Neg(Box<CountFormula>),
    Bin(char, Box<CountFormula>, Box<CountFormula>),
}

impl CountFormula {
    pub fn parse(s: &str) -> Result<CountFormula, String> {
        let tokens: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
        let mut pos = 0;
        let f = Self::sum(&tokens, &mut pos)?;
        if pos != tokens.len() {
            return Err(format!("unexpected '{}' in count formula '{s}'", tokens[pos]));
        }
        Ok(f)
    }

    fn sum(t: &[char], pos: &mut usize) -> Result<CountFormula, String> {
        let mut lhs = Self::product(t, pos)?;
        while let Some(&op @ ('+' | '-')) = t.get(*pos) {
            *pos += 1;
            lhs = CountFormula::Bin(op, Box::new(lhs), Box::new(Self::product(t, pos)?));
        }
        Ok(lhs)
    }

    fn product(t: &[char], pos: &mut usize) -> Result<CountFormula, String> {
        let mut lhs = Self::power(t, pos)?;
        while let Some(&op @ ('*' | '/')) = t.get(*pos) {
            *pos += 1;
            lhs = CountFormula::Bin(op, Box::new(lhs), Box::new(Self::power(t, pos)?));
        }
        Ok(lhs)
    }

    fn power(t: &[char], pos: &mut usize) -> Result<CountFormula, String> {
        let base = Self::atom(t, pos)?;
        if t.get(*pos) == Some(&'^') {
            *pos += 1;
            // Right associative.
            return Ok(CountFormula::Bin('^', Box::new(base), Box::new(Self::power(t, pos)?)));
        }
        Ok(base)
    }

    fn atom(t: &[char], pos: &mut usize) -> Result<CountFormula, String> {
        match t.get(*pos) {
            Some('(') => {
                *pos += 1;
                let f = Self::sum(t, pos)?;
                if t.get(*pos) != Some(&')') {
                    return Err("missing ')' in count formula".into());
                }
                *pos += 1;
                Ok(f)
            }
            Some('-') => {
                *pos += 1;
                Ok(CountFormula::Neg(Box::new(Self::atom(t, pos)?)))
            }
            Some('L' | 'l') => {
                *pos += 1;
                Ok(CountFormula::Level)
            }
            Some(c) if c.is_ascii_digit() => {
                let start = *pos;
                while t.get(*pos).is_some_and(|c| c.is_ascii_digit()) {
                    *pos += 1;
                }
                let s: String = t[start..*pos].iter().collect();
                s.parse().map(CountFormula::Num).map_err(|e| format!("{e}"))
            }
            other => Err(format!("unexpected {other:?} in count formula")),
        }
    }

    /// Evaluates for a level; saturates instead of overflowing.
    pub fn eval(&self, level: u32) -> i128 {
        match self {
            CountFormula::Num(n) => *n,
            CountFormula::Level => level as i128,
            CountFormula::Neg(a) => -a.eval(level),
            CountFormula::Bin(op, a, b) => {
                let (a, b) = (a.eval(level), b.eval(level));
                match op {
                    '+' => a.saturating_add(b),
                    '-' => a.saturating_sub(b),
                    '*' => a.saturating_mul(b),
                    '/' if b != 0 => a / b,
                    '^' if b >= 0 => a.checked_pow(b.min(u32::MAX as i128) as u32).unwrap_or(i128::MAX),
                    _ => 0,
                }
            }
        }
    }
}

impl ResearchUnit {
    /// Units needed for the given level.
    pub fn count_for(&self, level: u32) -> u64 {
        match &self.count {
            ResearchCount::Fixed(n) => *n,
            ResearchCount::Formula(f) => f.eval(level).clamp(1, u64::MAX as i128) as u64,
        }
    }
}

/// Names of the prototypes the simulation needs to treat specially. Taken from data,
/// e.g. the character is whatever entity has type `character` and name `character`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Special {
    pub character: Option<EntityProtoId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrototypeDb {
    pub items: Vec<ItemProto>,
    pub fluids: Vec<FluidProto>,
    pub recipes: Vec<RecipeProto>,
    pub entities: Vec<EntityProto>,
    pub tiles: Vec<TileProto>,
    pub technologies: Vec<TechnologyProto>,
    pub collision_layers: Vec<String>,
    pub special: Special,
    item_index: BTreeMap<String, ItemId>,
    fluid_index: BTreeMap<String, FluidId>,
    recipe_index: BTreeMap<String, RecipeId>,
    entity_index: BTreeMap<String, EntityProtoId>,
    tile_index: BTreeMap<String, TileId>,
    tech_index: BTreeMap<String, TechId>,
}

impl PrototypeDb {
    /// Builds the database. Each list must already be sorted by name, and ids used in
    /// cross references must be indices into these lists.
    pub fn new(
        items: Vec<ItemProto>,
        fluids: Vec<FluidProto>,
        recipes: Vec<RecipeProto>,
        entities: Vec<EntityProto>,
        tiles: Vec<TileProto>,
        collision_layers: Vec<String>,
    ) -> Self {
        fn index<T, I: Copy>(list: &[T], name: impl Fn(&T) -> &str, id: impl Fn(u16) -> I) -> BTreeMap<String, I> {
            assert!(list.windows(2).all(|w| name(&w[0]) < name(&w[1])), "prototype list must be sorted by name");
            list.iter().enumerate().map(|(i, t)| (name(t).to_owned(), id(i as u16))).collect()
        }
        let mut db = PrototypeDb {
            item_index: index(&items, |t| &t.name, ItemId),
            fluid_index: index(&fluids, |t| &t.name, FluidId),
            recipe_index: index(&recipes, |t| &t.name, RecipeId),
            entity_index: index(&entities, |t| &t.name, EntityProtoId),
            tile_index: index(&tiles, |t| &t.name, TileId),
            tech_index: BTreeMap::new(),
            technologies: Vec::new(),
            items,
            fluids,
            recipes,
            entities,
            tiles,
            collision_layers,
            special: Special::default(),
        };
        db.special.character = db
            .entity_index
            .get("character")
            .copied()
            .filter(|id| matches!(db.entity(*id).data, EntityData::Character { .. }))
            .or_else(|| {
                db.entities
                    .iter()
                    .position(|e| matches!(e.data, EntityData::Character { .. }))
                    .map(|i| EntityProtoId(i as u16))
            });
        db
    }

    /// Adds the technologies (sorted by name; ids in them index this list).
    pub fn set_technologies(&mut self, technologies: Vec<TechnologyProto>) {
        assert!(technologies.windows(2).all(|w| w[0].name < w[1].name), "technologies must be sorted by name");
        self.tech_index = technologies.iter().enumerate().map(|(i, t)| (t.name.clone(), TechId(i as u16))).collect();
        self.technologies = technologies;
    }

    pub fn technology(&self, id: TechId) -> &TechnologyProto {
        &self.technologies[id.index()]
    }
    pub fn technology_id(&self, name: &str) -> Option<TechId> {
        self.tech_index.get(name).copied()
    }
    pub fn technology_ids(&self) -> impl Iterator<Item = TechId> {
        (0..self.technologies.len() as u16).map(TechId)
    }

    pub fn item(&self, id: ItemId) -> &ItemProto {
        &self.items[id.index()]
    }
    pub fn fluid(&self, id: FluidId) -> &FluidProto {
        &self.fluids[id.index()]
    }
    pub fn recipe(&self, id: RecipeId) -> &RecipeProto {
        &self.recipes[id.index()]
    }
    pub fn entity(&self, id: EntityProtoId) -> &EntityProto {
        &self.entities[id.index()]
    }
    pub fn tile(&self, id: TileId) -> &TileProto {
        &self.tiles[id.index()]
    }

    pub fn item_id(&self, name: &str) -> Option<ItemId> {
        self.item_index.get(name).copied()
    }
    pub fn fluid_id(&self, name: &str) -> Option<FluidId> {
        self.fluid_index.get(name).copied()
    }
    pub fn recipe_id(&self, name: &str) -> Option<RecipeId> {
        self.recipe_index.get(name).copied()
    }
    pub fn entity_id(&self, name: &str) -> Option<EntityProtoId> {
        self.entity_index.get(name).copied()
    }
    pub fn tile_id(&self, name: &str) -> Option<TileId> {
        self.tile_index.get(name).copied()
    }

    pub fn item_ids(&self) -> impl Iterator<Item = ItemId> {
        (0..self.items.len() as u16).map(ItemId)
    }
    pub fn recipe_ids(&self) -> impl Iterator<Item = RecipeId> {
        (0..self.recipes.len() as u16).map(RecipeId)
    }
    pub fn entity_ids(&self) -> impl Iterator<Item = EntityProtoId> {
        (0..self.entities.len() as u16).map(EntityProtoId)
    }
    pub fn tile_ids(&self) -> impl Iterator<Item = TileId> {
        (0..self.tiles.len() as u16).map(TileId)
    }

    /// The item that places this entity (the first by name, like Factorio's
    /// `items_to_place_this`).
    pub fn item_to_place(&self, entity: EntityProtoId) -> Option<ItemId> {
        self.item_ids().find(|i| self.item(*i).place_result == Some(entity))
    }

    /// Recipes whose results include `item`, preferring the recipe named after the item.
    pub fn recipes_producing(&self, item: ItemId) -> Vec<RecipeId> {
        let name = &self.item(item).name;
        let mut out: Vec<RecipeId> = self
            .recipe_ids()
            .filter(|r| self.recipe(*r).results.iter().any(|p| p.what == ItemOrFluid::Item(item)))
            .collect();
        out.sort_by_key(|r| (self.recipe(*r).name != *name, *r));
        out
    }
}
