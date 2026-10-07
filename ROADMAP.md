# Roadmap

The goal is 1:1 gameplay parity with Factorio 2.0 (base game first, Space Age later), with
a deterministic simulation that can support lockstep multiplayer.

Each step ends with a playtest. Feedback from it becomes a ".5" step before the next one.

## Done

### Step 1: Foundation
- Cargo workspace: `factorio-sim` (deterministic, no Bevy or Lua), `factorio-data`
  (runs Factorio's Lua data stage from your install), `factorio-client` (Bevy).
- Fixed 60 UPS tick, fixed-point maths, state checksum.
- Data-driven prototypes so Space Age and mods can be enabled later.
- BUILDING.md for building with your own copy of the game.

### Step 2: Vertical slice
- Map generation (tiles, water, ore patches, starting area).
- Character: walking, hand mining, inventory, crafting queue with intermediates, building.
- Burner and electric drills, furnaces, assemblers, chests.
- Belts with lanes, curves, sideloading, undergrounds and splitters.
- Burner and electric inserters with the game's timing and insertion limits.
- Steam power: offshore pump, pipes, boiler, steam engine, poles, brownouts.
- Gameplay tests against real game values.

### Step 2.5: Playtest fixes
- Walking speed 8.9 tiles/s like the game; Factorio's default zoom.
- Curved belt sprites.
- Per-building power readouts; electric network window with power graph.
- Visual pass: all sprite layers, shadows, working animations, furnace fire.

## Next: Step 3, in order

1. **GUI parity.** Factorio's inventory, crafting, entity and character windows, quickbar,
   cursor stack, tooltips, and the same mouse and keyboard behaviour.
2. **Research.** Technology tree, labs, science packs, recipe unlocks and bonuses.
3. **Sound.** Game sounds from the install: building, mining, crafting, machine working
   loops, ambient sounds, UI sounds, with the game's falloff and volume settings.
4. **Map generation parity.** Implement Factorio's noise-expression language so maps
   generate from the real `autoplace` data: terrain, resources, trees, rocks, cliffs,
   decoratives, tile transitions.
5. **Inserters and poles.** Inserters reaching for moving belt items, stack bonuses, filter
   inserters; electric pole wiring as in the game (nearest-pole rules, copper wire).
6. **Fluids and oil.** Full fluid system, pumpjacks, oil refining, chemical plants, fluid
   recipes in assemblers.
7. **Trains and logistic robots.**
8. **Circuit network.**
9. **Combat.** Enemies, pollution, evolution, turrets, military items, health.
10. **Save, load and replays.**

## Step 4: Single-player polish

1. **Main menu.** Title screen, new game with map settings (seed, resource and terrain
   sliders), load game, settings (controls, graphics, sound), quit.
2. **Polish.** Rebindable controls (including the optional right-click clear-cursor binding),
   remaining visual effects (lights, smoke, tile transitions), GUI skin from the game's
   sprite sheets.
3. **Performance.** Large factories at 60 UPS.
4. **Space Age readiness.** Make sure the data-driven systems can take the DLC's prototypes.

## Later

- Space Age (as a mod set on the same data-driven systems).
- Multiplayer. The simulation keeps following the determinism rules (fixed tick, fixed
  point, stable ordering, inputs only) so lockstep multiplayer can be added.

The order can change based on playtesting and priorities.
