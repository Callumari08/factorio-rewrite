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
1. **GUI parity (in progress).** Every window and panel we have, each to be compared
   side by side with a capture from the real game (`scratchpad` capture mod `guishot`:
   one run shoots all of them; real Factorio is launched only with Callum's go-ahead).
   [x] = matches its capture; [~] = rebuilt in the game's layout, details left.
   - Shared: [x] skin from `gui-style` (frames, deep/shallow panels, slots, tabs, bars),
     [x] window frame with title bar, draggable filler and close button, [~] fonts and
     text styles, [ ] drag-spreading with the cursor stack, [x] hand slot, [ ] slot
     details (quality, filters, progress bars on slots), [ ] keyboard/mouse audit.
   - Character window (E): [x] inventory panel, [x] crafting panel (item-group tabs,
     subgroup rows, empty cells as in the game), [ ] logistics, armor and gun slots,
     [ ] inventory toolbar (sort, trash).
   - Entity windows: [x] assembling machines, [x] furnaces, [~] chests (inventory-limit
     button, deep frame), [x] mining drills, [~] inserters (filters, stack size
     override), [x] transport belt, [~] splitter (priorities, filter), [x] boiler,
     offshore pump, pipe, [~] electric network info (long time ranges), [x] lab.
   - Hover info panel (the selected entity's details at the side): [ ] all entities.
   - Tooltips: [ ] item, [ ] recipe (ingredients, time, totals, hints), [ ] technology,
     [ ] entity.
   - HUD: [x] quickbar, [x] crafting queue, [x] research progress (top right),
     [~] minimap (no zoom, no player name), [x] shortcut bar (buttons do nothing yet),
     [ ] alerts panel above the shortcut bar (red triangle with "!" while a building
     takes damage, "x" once destroyed, yellow for other alerts, with counts; also
     shown on the minimap) once combat exists.
   - Technology: [~] 2.0 tech screen (queue, card, list, tree; no drag/zoom/search yet).
   - Menus: [x] sound settings, [ ] the rest of the options menu, [ ] map view (M).
   - [ ] Pixel calibration pass: every widget measured against real-game screenshots
     (not photos) at a known UI scale, fixing sizes, offsets and text placement, e.g.
     the mining strip on the quickbar (Callum: not 1:1 yet).

## Next: the rest of step 3, in order
5. **Inserters and poles.** Done: inserter hand size with research bonuses, whole-stack
   pickup and drops, belt pickup while items arrive, filters, stack size override, the
   inserter window; copper wires as game state with the game's auto-connect rules
   (closest first, no triangles, at most 5), drawn sagging. Left: wiring by hand with
   copper wire (click poles, Shift+click to disconnect), inserters chasing moving belt
   items within the pickup area, wait_for_full_hand circuit control.
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
