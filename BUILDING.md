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

In the window: `1`-`5` choose an entity, left click places it, right click removes it,
`WASD` pans and `Q`/`E` zoom. The overlay shows the tick count, measured UPS and the
simulation checksum.

## 3. Pointing it at your Factorio install

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

## 4. Checking that the data loads, without a window

```sh
cargo run -p factorio-data --example dump
cargo run -p factorio-data --example dump -- recipe electronic-circuit
```

This runs Factorio's data stage and prints prototype counts, or one prototype.

## 5. Tests

```sh
cargo test --workspace
```

## Never commit game files

`.gitignore` excludes images, sounds, `/data`, `/factorio` and local config, but please check
your commits too. Anything from the game install stays on your machine.

## Troubleshooting

- **`could not find a Factorio install`**: set `FACTORIO_PATH` as described above.
- **Lots of `VALIDATION` errors from `wgpu_hal::vulkan` in debug builds**: these come from Vulkan
  validation layers installed on your system, not from this project. They are harmless and
  do not appear in release builds.
- **Screenshots for bug reports**: `FACTORIO_REWRITE_SCREENSHOT=shot.png cargo run` saves a
  screenshot after about three seconds and exits.
