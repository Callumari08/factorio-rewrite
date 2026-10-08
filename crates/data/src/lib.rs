//! Loads Factorio's game data from a user's own install.
//!
//! Nothing from the game is bundled with this project. At runtime we:
//!
//! 1. find the install ([`install`]),
//! 2. discover enabled mods (`core`, `base`, and later Space Age / user mods) and sort them
//!    into Factorio's load order ([`mods`]),
//! 3. run the settings and data stages in a Lua 5.2 VM with Factorio's own `dataloader.lua`
//!    and `lualib` ([`datastage`]), producing `data.raw` as a [`raw::RawValue`] tree,
//! 4. convert that into simulation-ready, fixed-point prototypes ([`typed`]).

pub mod datastage;
pub mod defines;
pub mod install;
pub mod locale;
pub mod mapgen;
pub mod mods;
pub mod raw;
pub mod sound;
pub mod sprite;
pub mod typed;

use std::path::PathBuf;

pub use datastage::{GameData, load_game_data};
pub use install::{Config, FactorioInstall};
pub use raw::RawValue;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "could not find a Factorio install; tried: {tried:?}. Set FACTORIO_PATH or factorio_path in factorio-rewrite.toml"
    )]
    InstallNotFound { tried: Vec<PathBuf> },
    #[error("I/O error at {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("invalid JSON in {path}: {source}")]
    Json { path: PathBuf, source: serde_json::Error },
    #[error("invalid config file {path}: {source}")]
    Config { path: PathBuf, source: toml::de::Error },
    #[error("mod error: {0}")]
    Mod(String),
    #[error("Lua error: {0}")]
    Lua(#[from] mlua::Error),
    #[error("prototype {kind}/{name}: {message}")]
    Prototype { kind: String, name: String, message: String },
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn read(path: &std::path::Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|source| Error::Io { path: path.to_owned(), source })
}
