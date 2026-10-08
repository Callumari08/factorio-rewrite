//! Deterministic Factorio simulation core.
//!
//! Rules every module in this crate must follow so lockstep multiplayer stays possible:
//!
//! * The simulation advances only in whole ticks ([`TICKS_PER_SECOND`] = 60 UPS).
//! * No `f32`/`f64` in game state or game logic. Use [`fixed::Fixed`] and the integer
//!   map types in [`map`]. Floats from prototype data are converted once, at load time.
//! * No `HashMap`/`HashSet` iteration in game logic (iteration order is randomised).
//!   Use `Vec`, `BTreeMap`, or id-ordered storage.
//! * No wall-clock time, threads, or OS randomness. Randomness comes from [`rng::DetRng`]
//!   stored in the simulation state.
//! * All external influence enters through [`input::InputAction`]s applied at a tick.
//!
//! This crate does not depend on Bevy or Lua; rendering and data loading live elsewhere.

pub mod belt;
pub mod cursor;
pub mod energy;
pub mod fixed;
pub mod input;
pub mod inventory;
pub mod machines;
pub mod map;
pub mod mapgen;
pub mod noise;
pub mod player;
pub mod power;
pub mod proto;
pub mod research;
pub mod rng;
pub mod surface;
pub mod world;

pub use fixed::Fixed;
pub use input::{InputAction, PlayerInput};
pub use proto::PrototypeDb;
pub use world::{Simulation, Tick};

/// Factorio's fixed update rate.
pub const TICKS_PER_SECOND: u32 = 60;
