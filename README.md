# factorio-rewrite

An open-source reimplementation of the Factorio engine in Rust with [Bevy](https://bevy.org),
aiming for 1:1 gameplay parity with Factorio 2.0.

**You need your own legal copy of Factorio.** This repository contains no game data, graphics,
sounds or Lua from the game. At startup it reads everything from your install, the same way
Factorio itself does. See [BUILDING.md](BUILDING.md).

This is a fan project and is not affiliated with or endorsed by Wube Software.

## Status

Step 1 (foundation):

- Cargo workspace with the simulation fully separated from rendering.
- Deterministic simulation core with a fixed 60 UPS tick, fixed-point maths and a state checksum.
- Prototype loader that runs Factorio's real settings and data stages (`core` + `base`) in
  embedded Lua 5.2, then converts `data.raw` into typed, fixed-point prototypes.
- Bevy client that opens a window, runs the sim, and draws entity and item sprites loaded
  straight from the install.

## Layout

| Crate | Path | What it does |
|-------|------|--------------|
| `factorio-sim` | `crates/sim` | Deterministic game state and tick. No Bevy, no Lua, no floats in game logic. |
| `factorio-data` | `crates/data` | Finds the install, orders mods, runs the Lua data stage, builds prototypes. |
| `factorio-client` | `crates/client` | Bevy app (binary `factorio-rewrite`): input, rendering, UI. |

## Design rules

- **Determinism first.** Lockstep multiplayer needs every peer to compute the same state from
  the same inputs. The `factorio-sim` crate uses integers and fixed point only, iterates in a
  stable order (`BTreeMap`, id-ordered storage, never `HashMap`), has its own seeded RNG, and
  changes only through `InputAction`s applied at a tick. `Simulation::checksum()` exists for
  desync detection.
- **Data driven.** Prototypes come from Factorio's own Lua data stage, and prototype categories
  come from the game's `defines.prototypes`, so Space Age and other mods can be added by
  enabling them rather than by changing code.
- **Rendering is a view.** The client reads the simulation and sends inputs. It never changes
  game state directly.
