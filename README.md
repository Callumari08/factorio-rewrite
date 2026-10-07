# factorio-rewrite

An open-source reimplementation of the Factorio engine in Rust with [Bevy](https://bevy.org),
aiming for 1:1 gameplay parity with Factorio 2.0.

**You need your own legal copy of Factorio.** This repository contains no game data, graphics,
sounds or Lua from the game. At startup it reads everything from your install, the same way
Factorio itself does. See [BUILDING.md](BUILDING.md).

This is a fan project and is not affiliated with or endorsed by Wube Software.

## Status

**Step 2 (vertical slice), playable:** a freeplay-style start on a generated map, through
the burner phase to steam power and assemblers.

- Map generation: tiles, water and ore patches (iron, copper, coal, stone) with a
  guaranteed starting area. Uses our own deterministic noise for now (see below).
- Character: walking with collision, hand mining (resources and buildings), an 80-slot
  inventory, and a hand-crafting queue that crafts missing intermediates.
- Burner and electric mining drills, stone furnaces, assembling machines, chests.
- Transport belts with two lanes, curves, sideloading, underground belts and splitters.
- Burner and electric inserters with the game's swing timing and insertion limits.
- Steam power: offshore pump, pipes, boiler, steam engine, small electric poles, with
  brownouts slowing machines down in proportion.

Behaviour is checked by tests against known game values (`crates/data/tests/gameplay.rs`).
For example: hand mining iron takes 120 ticks, a burner drill makes one ore every 240 ticks,
a furnace makes an iron plate every 192 ticks, a belt moves 15 items/s, and burner and
electric inserters take 76 and 70 ticks per swing.

Not done yet: research (every recipe is available), Factorio's own noise-expression map
generator, trees, rocks, cliffs and enemies, oil and fluid recipes, labs, combat, vehicles,
trains, logistics robots, circuits, blueprints, and saving and loading.

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

## License

Copyright (C) 2026 the factorio-rewrite contributors.

This program is free software: you can redistribute it and/or modify it under the terms of
the GNU General Public License as published by the Free Software Foundation, either version 3
of the License, or (at your option) any later version. See [LICENSE](LICENSE).

The license covers this project's own code only. Factorio and its data, graphics and sounds
are the property of Wube Software and are not part of this repository.
