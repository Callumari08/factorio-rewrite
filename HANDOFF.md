# Handoff

Read this first when picking the project up in a new session. See also
[ROADMAP.md](ROADMAP.md) (plan), [BUILDING.md](BUILDING.md) (build, run, controls) and
[README.md](README.md).

## Current state (2026-10-08)

- Steps 1, 2 and 2.5 are done. Step 3 item 1 (GUI parity) is done apart from the exact GUI
  skin, drag-spreading items across slots, and the hover info panel.
- Step 3 item 2 (research) is done: `crates/sim/src/research.rs` (force research state,
  queue, triggers, labs, bonuses), the technology window in `crates/client/src/ui/research.rs`.
  Bonuses that change existing mechanics are applied (character mining/crafting/running
  speed, inventory slots, lab speed and productivity, drill productivity); the others
  (inserter capacity, ammo damage, robots, ...) are stored in `Research::modifiers` for the
  systems that will use them. Unconfirmed against the game: inserters keep 2 packs per lab
  slot (`LAB_AUTOMATED_LIMIT`), and trigger counts only start once a technology is available.
- Step 3 item 3 (sound) is done: `crates/data/src/sound.rs` (parsing, `config.ini` settings),
  `crates/client/src/sound.rs` (playback). The sim reports `GameEvent`s (built, mined,
  crafted, research finished) for it; they are not game state. Approximations: the distance
  falloff curve and hearing range (not documented by the game), footstep and pickaxe timing,
  wind/ambience crossfade, music order, idle sounds and sound accents are not played.
- Step 3 item 4 (map generation parity) is in progress. `crates/sim/src/mapgen/` parses,
  compiles and evaluates the game's noise expressions (all of Nauvis's compile);
  `crates/data/src/mapgen.rs` `planet_mapgen` feeds it the planet's tiles and resources.
  It is the default (`FACTORIO_REWRITE_SIMPLE_MAPGEN=1` gives the old generator). Spot
  noise regions are centred on the origin, which keeps each starting patch whole. Chunks
  beyond the character's neighbours generate one per tick. `cargo run --release -p
  factorio-data --example mapimage -- map.ppm 24 [seed]` renders a map for checking.
  Trees and rocks are generated (`EntityState::Static`, minable, collidable; trees come
  from the `trees` autoplace control). The renderer now only mirrors on-screen entities.
  Ground (`terrain.rs`): 32 px/tile textures near the camera, 8 px/tile for every visible
  chunk (used when zoomed out), composed on the CPU within a per-frame time budget. Tile
  edges use `variants.transition` masks; shores use the land tiles' `transitions` to
  water (mask, bank overlay, background; two-tile-tall pieces); water and deep water blend
  with a soft ramp. Not done: the animated water shader (`tile-effect` "water"), effect
  maps/foam. Entities and cliffs are only drawn on chunks whose ground is ready.
  The world generates 7 chunks around each character (3 at once, the rest one per tick).
  Reference maps: `<factorio>/bin/x64/factorio --generate-map-preview out.png
  --map-gen-seed N --report-quantities iron-ore,...` gives the real game's map and ore
  totals for calibration.
  Decoratives are generated on demand by the client (`Surface::decoratives`, not game
  state) and painted into the chunk ground textures; building does not remove them yet.
  Cliffs: marching squares over `cliff_elevation` on the cliff's 4x4 grid where
  `cliffiness` > 0.5, stored per chunk (not in the entity map, so they need no
  footprint tiles); `Simulation::cliff_in` blocks walking and building. Cliff explosives
  are not implemented. The left-hand-high orientation convention is inferred.
  Still to do: water shore sprites, fish, crude oil, enemies.
  Factorio's basis noise algorithm is not public; ours is gradient noise scaled to about
  -1..1 (`BASIS_AMPLITUDE`), and `multioctave_noise`'s loudness (`MULTIOCTAVE_GAIN` = 3) is
  calibrated so lakes cover 10-20% of the map, so maps follow the game's rules but not its
  seeds. Moisture varies a lot between seeds (some spawn in dirt or sand).
- All tests pass: `cargo test --workspace` (sim unit tests, plus gameplay tests in
  `crates/data/tests/` that load the real game and check values such as tick timings and
  throughputs).

## Crate map

| Crate | What lives there |
|-------|------------------|
| `crates/sim` (`factorio-sim`) | All game rules. `world.rs` (Simulation, entities, building, checksum), `player.rs` (character, mining, crafting queue, inputs), `cursor.rs` (cursor stack, slot clicks), `machines.rs` (drills, furnaces/assemblers, inserters), `belt.rs` (lanes, curves, sideload, undergrounds, splitters), `power.rs` (fluid segments, electric networks, power stats), `research.rs` (technologies, queue, triggers, labs, bonuses), `surface.rs` + `noise.rs` (chunks, map generation), `proto.rs` (typed prototypes, ids), `input.rs` (every player action). |
| `crates/data` (`factorio-data`) | Finds the install, orders mods, runs the Lua settings/data stages (`datastage.rs`), converts `data.raw` to typed prototypes (`typed.rs`), map-gen settings (`mapgen.rs`), sprite lookups (`sprite.rs`), locale names (`locale.rs`). |
| `crates/client` (`factorio-client`, binary `factorio-rewrite`) | Bevy app: `main.rs` (setup, fixed 60 Hz tick), `controls.rs` (keys/mouse → inputs), `render.rs` (entities, layers, belts, items, ghost), `terrain.rs`, `ui.rs` (all GUI; `ui/research.rs` the technology window and lab panel), `chart.rs` (power graph), `sound.rs` (all audio), `demo.rs`. |

## Conventions (keep these)

- **Determinism.** Sim: fixed 60 UPS, no floats in game logic (use `Fixed`, integer map
  positions in 1/256 tiles), no HashMap iteration, no wall clock or threads, all changes via
  `InputAction`s. The one float exception is map generation, which evaluates noise in `f32` like
  the game, using only IEEE basic operations and the pure-Rust `libm` (bit-identical on all
  platforms). Times are stored as exact ticks at load time (e.g. `mining_ticks`).
- **Data-driven.** No base-game names in the sim; prototypes and categories come from the
  game data (`defines.prototypes`, collision mask defaults, locale) so Space Age/mods work.
- **Never commit game data.** Everything is read from the user's install at runtime.
- **Parity is tested** against known game values; add a test for each mechanic.
- **Visual checks:** short screenshot runs (`FACTORIO_REWRITE_SCREENSHOT=...png
  FACTORIO_REWRITE_SCREENSHOT_AFTER=4`) are fine on Callum's desktop; keep them brief.
- Send Callum short progress notes every few minutes during long work; small commits,
  pushed to `main`.

## Known gaps

Map generation (decoratives, cliffs, shores, fish, oil, enemies); exact GUI skin; the technology window is a grid, not the game's tree view; lights, smoke; inserters chasing belt items and real pole
wiring; fluid recipes; trains, robots, circuits, combat; save/load; main menu; rebindable
controls; data-stage `pairs()` order and `math.random` are not identical to the game.
