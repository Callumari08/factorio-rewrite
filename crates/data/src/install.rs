//! Finding the user's Factorio install and the project config file.

use std::env;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Error, Result};

/// Optional `factorio-rewrite.toml`, looked up in the current directory, then in
/// `$XDG_CONFIG_HOME/factorio-rewrite/` (or the platform equivalent).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Root of the Factorio install (the directory containing `data/`).
    pub factorio_path: Option<PathBuf>,
    /// Mods to enable in addition to `core` and `base`, e.g. `["elevated-rails", "quality", "space-age"]`.
    #[serde(default)]
    pub mods: Vec<String>,
    /// Extra directories to search for unpacked mods (each mod in its own folder with `info.json`).
    #[serde(default)]
    pub mod_dirs: Vec<PathBuf>,
}

impl Config {
    pub fn load() -> Result<Config> {
        let Some(path) = Self::find_file() else { return Ok(Config::default()) };
        let text = std::fs::read_to_string(&path).map_err(|source| Error::Io { path: path.clone(), source })?;
        let config: Config = toml::from_str(&text).map_err(|source| Error::Config { path: path.clone(), source })?;
        log::info!("using config {}", path.display());
        Ok(config)
    }

    fn find_file() -> Option<PathBuf> {
        if let Some(p) = env::var_os("FACTORIO_REWRITE_CONFIG") {
            return Some(PathBuf::from(p));
        }
        let mut candidates = vec![PathBuf::from("factorio-rewrite.toml")];
        if let Some(dir) = config_dir() {
            candidates.push(dir.join("factorio-rewrite").join("factorio-rewrite.toml"));
        }
        candidates.into_iter().find(|p| p.is_file())
    }
}

pub(crate) fn config_dir() -> Option<PathBuf> {
    if let Some(x) = env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(x));
    }
    if cfg!(windows) {
        return env::var_os("APPDATA").map(PathBuf::from);
    }
    let home = PathBuf::from(env::var_os("HOME")?);
    Some(if cfg!(target_os = "macos") { home.join("Library/Application Support") } else { home.join(".config") })
}

#[derive(Clone, Debug)]
pub struct FactorioInstall {
    /// The install root.
    pub root: PathBuf,
    /// `<root>/data`, containing `core`, `base` and any bundled DLC mods.
    pub data_dir: PathBuf,
    /// Version string from `base/info.json`, e.g. `2.0.77`.
    pub version: String,
}

impl FactorioInstall {
    /// Resolution order: `FACTORIO_PATH` env var, `factorio_path` in the config, then the
    /// common Steam / standalone install locations for the current platform.
    pub fn locate(config: &Config) -> Result<FactorioInstall> {
        let mut tried = Vec::new();
        let explicit = env::var_os("FACTORIO_PATH").map(PathBuf::from).or_else(|| config.factorio_path.clone());
        let candidates = match explicit {
            Some(p) => vec![p],
            None => default_candidates(),
        };
        for root in candidates {
            if let Some(install) = Self::at(&root)? {
                return Ok(install);
            }
            tried.push(root);
        }
        Err(Error::InstallNotFound { tried })
    }

    /// Checks whether `root` (or the macOS app bundle inside it) is a Factorio install.
    pub fn at(root: &Path) -> Result<Option<FactorioInstall>> {
        for root in [root.to_path_buf(), root.join("factorio.app/Contents")] {
            let info = root.join("data/base/info.json");
            if info.is_file() {
                let bytes = crate::read(&info)?;
                let info: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|source| Error::Json { path: info.clone(), source })?;
                let version = info["version"].as_str().unwrap_or("unknown").to_owned();
                return Ok(Some(FactorioInstall { data_dir: root.join("data"), root, version }));
            }
        }
        Ok(None)
    }

    /// Factorio's user data directory, where `mods/` lives.
    pub fn user_dir() -> Option<PathBuf> {
        if cfg!(windows) {
            return env::var_os("APPDATA").map(|p| PathBuf::from(p).join("Factorio"));
        }
        let home = PathBuf::from(env::var_os("HOME")?);
        Some(if cfg!(target_os = "macos") {
            home.join("Library/Application Support/factorio")
        } else {
            home.join(".factorio")
        })
    }

    /// `doc-html/runtime-api.json`, which ships with the game and describes `defines`.
    pub fn runtime_api_json(&self) -> PathBuf {
        self.root.join("doc-html/runtime-api.json")
    }
}

fn default_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let home = env::var_os("HOME").map(PathBuf::from);
    if cfg!(windows) {
        for base in ["C:\\Program Files (x86)\\Steam", "C:\\Program Files\\Steam"] {
            out.push(PathBuf::from(base).join("steamapps\\common\\Factorio"));
        }
        out.push(PathBuf::from("C:\\Program Files\\Factorio"));
    } else if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            out.push(h.join("Library/Application Support/Steam/steamapps/common/Factorio"));
        }
        out.push(PathBuf::from("/Applications/factorio.app/Contents"));
    } else if let Some(h) = &home {
        out.push(h.join(".local/share/Steam/steamapps/common/Factorio"));
        out.push(h.join(".steam/steam/steamapps/common/Factorio"));
        out.push(h.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/common/Factorio"));
        out.push(h.join("factorio"));
    }
    out
}
