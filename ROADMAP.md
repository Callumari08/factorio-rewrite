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

### Step 3, so far
2. **Research.** Technologies from the game data, labs and science packs, the research
   queue, Factorio 2.0 research triggers, recipe unlocks, bonuses, infinite technologies
   and a first technology window.
3. **Sound.** Game sounds from the install: machine working loops, building, mining,
   crafting, research, footsteps, GUI and inventory sounds, zoom-dependent ambience
   (wind/base ambience crossfade), music, and a volume panel starting from the player's
   Factorio settings.

## In progress

4. **Map generation parity.** Done: the noise-expression language (parser, compiler,
   evaluator, spot noise), tiles, lakes, ores, trees, rocks, decoratives, cliffs, tile
   transitions, GPU ground rendering (no seams, no flicker), the game's zoom range with
   the ground generated ahead of the furthest view. Later: a closer match to the game's
   terrain look (basis noise and multioctave calibration); ore calibration (richness is
   about 73% of the game's, and rare ores such as uranium are far too common: compare
   per-ore patch counts and amounts with `--generate-map-preview --report-quantities`);
   the animated water shader; zooming out past 0.3 into the map view.
1. **GUI parity (reopened; first pass only).** Checklist, each compared side by side with
   the game:
   - [ ] GUI skin from the game's own sprite sheets (`__core__/graphics/gui*`): frames,
         title bars, inner panels, slot buttons, tabs, scrollbars, buttons, progress bars.
   - [ ] Exact window layouts and sizes at the default UI scale: character window
         (character, inventory, crafting with item-group tabs and subgroup rows, logistics
         and armor sections), entity windows, quickbar, crafting queue, research HUD.
   - [ ] Fonts and text styles as the game's GUI styles (sizes, colours, shadows).
   - [ ] Tooltips laid out as in the game (recipe/item/entity tooltips with icons,
         ingredients, totals, crafting time, "Ctrl+click" hints).
   - [ ] Entity hover info panel (right side "selected entity" info).
   - [ ] Drag-spreading items across slots with the cursor stack (left and right drag).
   - [ ] Technology tree view as in 2.0 (graph of technologies, zoom, search) instead of
         the grid, with the game's tech slot styles and queue.
   - [ ] Slot details: count fonts, red/green slot states, filters, hover highlight,
         item quality/progress bars on slots.
   - [ ] Minimap and map view (M), alerts panel position, shortcut bar.
   - [ ] Keyboard and mouse behaviour audit against the game's default controls.

## Next: the rest of step 3, in order
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
