# Building and running

## 1. What you need

- **Factorio 2.0**, installed from Steam or factorio.com. This project reads the game's data
  from your install at runtime and does not ship any of it. Base game only for now (Space
  Age is not required).
- **Rust**, stable, 1.89 or newer. Install it with [rustup](https://rustup.rs).
- **A C compiler.** Lua 5.2 is compiled from source as part of the build (gcc, clang or MSVC).
- **Bevy's system dependencies:**
  - Debian/Ubuntu: `sudo apt install g++ pkg-config libx11-dev libasound2-dev libudev-dev libxkbcommon-x11-0 libwayland-dev libxkbcommon-dev`
  - Fedora: `sudo dnf install gcc-c++ libX11-devel alsa-lib-devel systemd-devel wayland-devel libxkbcommon-devel`
  - Arch: `sudo pacman -S base-devel alsa-lib systemd-libs wayland libxkbcommon`
  - Windows and macOS need nothing extra.
  - Full list: <https://github.com/bevyengine/bevy/blob/main/docs/linux_dependencies.md>

## 2. Build and run

```sh
git clone https://github.com/Callumari08/factorio-rewrite
cd factorio-rewrite
cargo run --release
```

The first build takes a few minutes because it compiles Bevy. Plain `cargo run` (a debug
build) also works and is quick enough, because dependencies are optimised even in debug.

See [Playing](#3-playing) for the controls.

## 3. Playing

You start like Factorio's freeplay: 8 iron plates, a burner mining drill, a stone furnace
and a pistol, next to patches of iron, copper, coal and stone. Only the game's starting
recipes are unlocked. As in Factorio 2.0, smelting 50 iron plates researches Steam power,
10 copper plates Electronics, and crafting a lab Automation science pack; after that, labs
research the technologies you queue with `T`.

| Input | Action |
|-------|--------|
| `W` `A` `S` `D` | Walk (diagonals too) |
| Right mouse (hold) | Mine the building or resource under the cursor (also while holding an item) |
| `E` | Open/close the character window (inventory and crafting) |
| `T` | Open/close the technology window. Click a technology for details, then Start research (Shift+click puts it at the front of the queue). Right click a queued technology to remove it |
| Left click on a slot | Pick up / put down / swap the stack |
| Right click on a slot | Take half / put down one |
| `Shift`+click / `Ctrl`+click on a slot | Move a stack / all of that item to the open building (or back) |
| Left mouse in the world | Build the held item (drag for lines), or open a building |
| `Ctrl`+left / `Ctrl`+right on a building | Insert the held stack / half of it, or take its output |
| `R` / `Shift`+`R` | Rotate the held item or the building under the cursor |
| `Q` | Put the held stack back, or pick the item for the building under the cursor |
| `1` – `0` | Pick up the quickbar item. Click a quickbar slot while holding an item to assign it; right click clears it |
| `F` (hold) | Pick up items from the ground and nearby belts |
| Recipe: left / right / `Shift`+click | Craft 1 / 5 / as many as possible. Click a queued craft (bottom left) to cancel it |
| Mouse wheel | Zoom |
| `F1` | Sandbox: a full stack of every item (what doesn't fit goes into chests next to you) |
| `F2` | Toggle cheat mode: crafting is instant and free (like the game's `/cheat`) |
| `F3` | Research every technology |

Buildings show their slots in their window: click with a held stack to put fuel, ore or
ingredients in, and click the result slot to take output. Assemblers show a recipe chooser.

Optional environment variables:

- `FACTORIO_REWRITE_DEMO=1` builds a small burner factory (drills → belt → inserter →
  furnace → inserter → chest) on the nearest iron patch, plus steam power, an assembler and
  a lab researching Automation.
- `FACTORIO_REWRITE_UI=1` (or `power`, `lab`, `tech`) opens a window at start, for screenshots;
  `FACTORIO_REWRITE_TECH=<name>` selects a technology in the technology window.
- `FACTORIO_REWRITE_SEED=1234` picks a different map.
- `FACTORIO_REWRITE_SCREENSHOT=shot.png` (with `FACTORIO_REWRITE_SCREENSHOT_AFTER=10`)
  saves a screenshot after that many seconds and exits.

## 4. Pointing it at your Factorio install

The game is looked up in this order:

1. The `FACTORIO_PATH` environment variable.
2. `factorio_path` in `factorio-rewrite.toml`. The file is read from the current directory, then
   from `~/.config/factorio-rewrite/` (Linux), `~/Library/Application Support/factorio-rewrite/`
   (macOS) or `%APPDATA%\factorio-rewrite\` (Windows). `FACTORIO_REWRITE_CONFIG` can name the
   file directly. See [`factorio-rewrite.example.toml`](factorio-rewrite.example.toml).
3. Common install locations:
   - Linux: `~/.local/share/Steam/steamapps/common/Factorio`, `~/.steam/steam/steamapps/common/Factorio`,
     the Flatpak Steam path, and `~/factorio`.
   - Windows: `C:\Program Files (x86)\Steam\steamapps\common\Factorio`, `C:\Program Files\Factorio`.
   - macOS: `~/Library/Application Support/Steam/steamapps/common/Factorio` and `/Applications/factorio.app`.

Set the path to the folder that contains `data/base/info.json`. On macOS the folder holding
`factorio.app` also works. Examples:

```sh
FACTORIO_PATH="$HOME/games/factorio" cargo run --release
```

```powershell
$env:FACTORIO_PATH = "D:\SteamLibrary\steamapps\common\Factorio"; cargo run --release
```

The loader also needs `doc-html/runtime-api.json` from the install, which comes with the
Steam and standalone versions of the game. It supplies the `defines` table to the data stage.

## 5. Checking that the data loads, without a window

```sh
cargo run -p factorio-data --example dump
cargo run -p factorio-data --example dump -- recipe electronic-circuit
```

This runs Factorio's data stage and prints prototype counts, or one prototype.

## 6. Tests

```sh
cargo test --workspace
```

The gameplay tests in `crates/data/tests/gameplay.rs` load your Factorio install and check
timings and throughputs against the real game. Without an install they are skipped.

## Never commit game files

`.gitignore` excludes images, sounds, `/data`, `/factorio` and local config, but please check
your commits too. Anything from the game install stays on your machine.

## Troubleshooting

- **`could not find a Factorio install`**: set `FACTORIO_PATH` as described above.
- **Lots of `VALIDATION` errors from `wgpu_hal::vulkan` in debug builds**: these come from Vulkan
  validation layers installed on your system, not from this project. They are harmless and
  do not appear in release builds.
- **Screenshots for bug reports**: `FACTORIO_REWRITE_SCREENSHOT=shot.png cargo run` saves a
  screenshot after four seconds and exits.
