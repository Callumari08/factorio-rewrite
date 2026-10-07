//! Simulation state and the fixed tick.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::input::{InputAction, PlayerInput};
use crate::map::{Direction, MapPosition};
use crate::proto::PrototypeDb;
use crate::rng::DetRng;

pub type Tick = u64;

/// Stable entity handle. Ids are allocated monotonically and never reused, so iterating
/// entities by id is a stable, platform-independent order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    pub prototype: String,
    pub position: MapPosition,
    pub direction: Direction,
}

/// The complete deterministic game state.
#[derive(Clone, Debug)]
pub struct Simulation {
    prototypes: Arc<PrototypeDb>,
    tick: Tick,
    rng: DetRng,
    next_entity_id: u64,
    entities: BTreeMap<EntityId, Entity>,
}

impl Simulation {
    pub fn new(prototypes: Arc<PrototypeDb>, seed: u64) -> Self {
        Simulation { prototypes, tick: 0, rng: DetRng::new(seed), next_entity_id: 1, entities: BTreeMap::new() }
    }

    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn prototypes(&self) -> &PrototypeDb {
        &self.prototypes
    }

    pub fn entities(&self) -> impl Iterator<Item = (EntityId, &Entity)> {
        self.entities.iter().map(|(id, e)| (*id, e))
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

        // Entity update phases (belts, inserters, crafting, ...) are added here in later
        // steps, each iterating in stable id order.

        self.tick += 1;
    }

    fn apply_input(&mut self, input: &PlayerInput) {
        match &input.action {
            InputAction::DebugPlaceEntity { prototype, position, direction } => {
                if !self.prototypes.entities.contains_key(prototype) {
                    return;
                }
                let id = EntityId(self.next_entity_id);
                self.next_entity_id += 1;
                self.entities
                    .insert(id, Entity { prototype: prototype.clone(), position: *position, direction: *direction });
            }
            InputAction::DebugRemoveEntity { position } => {
                let tile = position.tile();
                let hit = self.entities.iter().find(|(_, e)| e.position.tile() == tile).map(|(id, _)| *id);
                if let Some(id) = hit {
                    self.entities.remove(&id);
                }
            }
        }
    }

    /// A hash of the full game state, for desync detection between lockstep peers.
    /// Uses FNV-1a over a fixed field order, so it is stable across platforms and builds.
    pub fn checksum(&self) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.tick);
        h.u64(self.rng.state());
        h.u64(self.next_entity_id);
        for (id, e) in &self.entities {
            h.u64(id.0);
            h.bytes(e.prototype.as_bytes());
            h.u64(e.position.x as u32 as u64);
            h.u64(e.position.y as u32 as u64);
            h.u64(e.direction.0 as u64);
        }
        h.finish()
    }
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, bytes: &[u8]) {
        self.u64(bytes.len() as u64);
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed::Fixed;
    use crate::proto::EntityProto;

    fn db() -> Arc<PrototypeDb> {
        let mut db = PrototypeDb::default();
        db.entities.insert(
            "wooden-chest".into(),
            EntityProto {
                name: "wooden-chest".into(),
                kind: "container".into(),
                collision_box: [[-90, -90], [90, 90]],
                belt_speed: None::<Fixed>,
            },
        );
        Arc::new(db)
    }

    fn place(player: u16, x: i32) -> PlayerInput {
        PlayerInput {
            player,
            action: InputAction::DebugPlaceEntity {
                prototype: "wooden-chest".into(),
                position: MapPosition::from_tiles(x, 0),
                direction: Direction::NORTH,
            },
        }
    }

    #[test]
    fn input_order_does_not_change_outcome() {
        let mut a = Simulation::new(db(), 42);
        let mut b = Simulation::new(db(), 42);
        a.step(&[place(1, 0), place(2, 5)]);
        b.step(&[place(2, 5), place(1, 0)]);
        for _ in 0..600 {
            a.step(&[]);
            b.step(&[]);
        }
        assert_eq!(a.tick(), 601);
        assert_eq!(a.checksum(), b.checksum());
        assert_eq!(a.entities().count(), 2);
    }
}
