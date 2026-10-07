//! Runs Factorio's settings and data stages in an embedded Lua 5.2 VM.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use factorio_sim::rng::DetRng;
use mlua::{Lua, MultiValue, Table, Value};

use crate::defines::{self, DefineNode};
use crate::install::{Config, FactorioInstall};
use crate::mods::{self, ModInfo};
use crate::raw::RawValue;
use crate::{Error, Result};

const SETTINGS_STAGE: [&str; 3] = ["settings.lua", "settings-updates.lua", "settings-final-fixes.lua"];
const DATA_STAGE: [&str; 3] = ["data.lua", "data-updates.lua", "data-final-fixes.lua"];

/// Seed for `math.random` during the data stage (Factorio also uses a constant seed).
const DATA_STAGE_SEED: u64 = 0;

/// Everything produced by loading the game's data.
#[derive(Debug)]
pub struct GameData {
    pub install: FactorioInstall,
    /// Enabled mods in load order, `core` first.
    pub mods: Vec<ModInfo>,
    /// `settings.startup` as seen by the data stage.
    pub startup_settings: RawValue,
    /// `data.raw`: prototype type -> name -> definition.
    pub raw: RawValue,
    /// Prototype category -> member types (from `defines.prototypes`).
    pub categories: Vec<(String, Vec<String>)>,
}

impl GameData {
    pub fn prototype(&self, kind: &str, name: &str) -> &RawValue {
        self.raw.get(kind).get(name)
    }

    /// All prototypes of every `type` in a category such as `"item"` or `"entity"`,
    /// in a stable (type, name) order.
    pub fn prototypes_in_category<'a>(
        &'a self,
        category: &str,
    ) -> impl Iterator<Item = (&'a str, &'a str, &'a RawValue)> {
        let types: &[String] =
            self.categories.iter().find(|(c, _)| c == category).map(|(_, t)| t.as_slice()).unwrap_or(&[]);
        types.iter().flat_map(move |t| {
            self.raw.get(t).as_table().into_iter().flatten().map(move |(n, v)| (t.as_str(), n.as_str(), v))
        })
    }

    /// Resolves a Factorio asset path such as `__base__/graphics/icons/iron-plate.png`.
    pub fn resolve_path(&self, path: &str) -> Option<PathBuf> {
        let rest = path.strip_prefix("__")?;
        let end = rest.find("__")?;
        let (name, file) = (&rest[..end], rest[end + 2..].trim_start_matches('/'));
        let root = self.mods.iter().find(|m| m.name == name)?.root.clone();
        let full = root.join(file);
        safe_relative(Path::new(file)).then_some(full)
    }
}

/// Locates the install, resolves mods and runs both stages.
pub fn load_game_data(config: &Config) -> Result<GameData> {
    let install = FactorioInstall::locate(config)?;
    log::info!("Factorio {} at {}", install.version, install.root.display());
    let mods = mods::resolve_mods(&install, config)?;
    log::info!("mod load order: {:?}", mods.iter().map(|m| m.name.as_str()).collect::<Vec<_>>());
    let defines = defines::read_defines(&install.runtime_api_json())?;

    let settings_lua = new_stage_lua(&mods, &defines)?;
    let loader = Loader::install(&settings_lua, &install, &mods)?;
    loader.require_file(&settings_lua, &install.data_dir.join("core/lualib/dataloader.lua"), "core")?;
    loader.run_stage(&settings_lua, &SETTINGS_STAGE)?;
    let startup: Value = settings_lua.load(STARTUP_SETTINGS_LUA).set_name("=startup-settings").eval()?;
    let startup_settings = RawValue::from_lua(&startup);
    drop(settings_lua);

    let lua = new_stage_lua(&mods, &defines)?;
    let settings = lua.create_table()?;
    settings.set("startup", startup_settings.to_lua(&lua)?)?;
    lua.globals().set("settings", settings)?;
    let loader = Loader::install(&lua, &install, &mods)?;
    loader.require_file(&lua, &install.data_dir.join("core/lualib/dataloader.lua"), "core")?;
    loader.run_stage(&lua, &DATA_STAGE)?;
    let data: Table = lua.globals().get("data")?;
    let raw = RawValue::from_lua(&data.get::<Value>("raw")?);

    Ok(GameData { categories: defines::prototype_categories(&defines), install, mods, startup_settings, raw })
}

const STARTUP_SETTINGS_LUA: &str = r#"
local startup = {}
for _, t in ipairs{"bool-setting", "int-setting", "double-setting", "string-setting", "color-setting"} do
  for name, s in pairs(data.raw[t] or {}) do
    if s.setting_type == "startup" then
      local v = s.default_value
      if s.hidden and s.forced_value ~= nil then v = s.forced_value end
      startup[name] = { value = v }
    end
  end
end
return startup
"#;

fn new_stage_lua(mods: &[ModInfo], defines: &[DefineNode]) -> Result<Lua> {
    let lua = Lua::new();
    let g = lua.globals();

    lua.load(include_str!("prelude.lua")).set_name("=prelude").exec()?;

    let mods_table = lua.create_table()?;
    for m in mods.iter().filter(|m| m.name != "core") {
        mods_table.set(m.name.as_str(), m.version.as_str())?;
    }
    g.set("mods", mods_table)?;

    // Feature flags are switched on by any enabled mod declaring `<flag>_required: true`.
    let flags = lua.create_table()?;
    for flag in
        ["quality", "rail_bridges", "space_travel", "spoiling", "freezing", "segmented_units", "expansion_shaders"]
    {
        let key = format!("{flag}_required");
        let on = mods.iter().any(|m| m.extra.get(&key).and_then(|v| v.as_bool()) == Some(true));
        flags.set(flag, on)?;
    }
    g.set("feature_flags", flags)?;
    g.set("defines", defines::to_lua(&lua, defines)?)?;

    g.set(
        "log",
        lua.create_function(|_, v: Value| {
            log::debug!(target: "lua", "{}", v.to_string().unwrap_or_default());
            Ok(())
        })?,
    )?;
    g.set(
        "localised_print",
        lua.create_function(|_, v: Value| {
            println!("{}", v.to_string().unwrap_or_default());
            Ok(())
        })?,
    )?;

    // Deterministic math.random with a constant seed, as in Factorio's data stage.
    // TODO: match Factorio's exact generator so random-dependent data matches bit-for-bit.
    let rng = Rc::new(RefCell::new(DetRng::new(DATA_STAGE_SEED)));
    let math: Table = g.get("math")?;
    math.set(
        "random",
        lua.create_function(move |_, (m, n): (Option<f64>, Option<f64>)| {
            let mut rng = rng.borrow_mut();
            let (lo, hi) = match (m, n) {
                (None, _) => return Ok(Value::Number((rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64)),
                (Some(m), None) => (1, m.floor() as i64),
                (Some(m), Some(n)) => (m.floor() as i64, n.floor() as i64),
            };
            if hi < lo {
                return Err(mlua::Error::runtime("bad argument to 'random' (interval is empty)"));
            }
            let span = (hi - lo + 1) as u64;
            Ok(Value::Number((lo + (rng.next_u64() % span) as i64) as f64))
        })?,
    )?;
    math.set("randomseed", lua.create_function(|_, _: MultiValue| Ok(()))?)?;

    Ok(lua)
}

struct Frame {
    mod_name: String,
    dir: PathBuf,
}

/// Implements Factorio's `require`: paths relative to the current file, then to the
/// current mod's root, then `core/lualib`; `__mod-name__/...` (or `__mod-name__.x`)
/// addresses another mod. `..` is not allowed. Each file runs at most once per stage.
#[derive(Clone)]
struct Loader {
    mods: Rc<BTreeMap<String, PathBuf>>,
    order: Rc<Vec<String>>,
    lualib: Rc<PathBuf>,
    stack: Rc<RefCell<Vec<Frame>>>,
    loaded: Rc<RefCell<BTreeMap<PathBuf, mlua::RegistryKey>>>,
}

impl Loader {
    fn install(lua: &Lua, install: &FactorioInstall, mods: &[ModInfo]) -> Result<Loader> {
        let loader = Loader {
            mods: Rc::new(mods.iter().map(|m| (m.name.clone(), m.root.clone())).collect()),
            order: Rc::new(mods.iter().map(|m| m.name.clone()).collect()),
            lualib: Rc::new(install.data_dir.join("core/lualib")),
            stack: Rc::new(RefCell::new(Vec::new())),
            loaded: Rc::new(RefCell::new(BTreeMap::new())),
        };
        let l = loader.clone();
        let require = lua.create_function(move |lua, name: String| {
            let (path, mod_name) = l.resolve(&name).ok_or_else(|| {
                mlua::Error::runtime(format!(
                    "module {name} not found (searched relative to the current file, mod root and core/lualib)"
                ))
            })?;
            l.require_file(lua, &path, &mod_name).map_err(|e| match e {
                Error::Lua(e) => e,
                other => mlua::Error::runtime(other.to_string()),
            })
        })?;
        lua.globals().set("require", require)?;
        Ok(loader)
    }

    fn run_stage(&self, lua: &Lua, files: &[&str]) -> Result<()> {
        for file in files {
            for name in self.order.iter() {
                let path = self.mods[name].join(file);
                if path.is_file() {
                    log::debug!("running __{name}__/{file}");
                    self.require_file(lua, &path, name)?;
                }
            }
        }
        Ok(())
    }

    fn resolve(&self, name: &str) -> Option<(PathBuf, String)> {
        let name = name.strip_suffix(".lua").unwrap_or(name);
        let to_path = |s: &str| -> PathBuf {
            let s = if s.contains('/') { s.to_owned() } else { s.replace('.', "/") };
            PathBuf::from(format!("{s}.lua"))
        };

        if let Some(rest) = name.strip_prefix("__") {
            let end = rest.find("__")?;
            let mod_name = &rest[..end];
            let rel = to_path(rest[end + 2..].trim_start_matches(['/', '.']));
            if !safe_relative(&rel) {
                return None;
            }
            let path = self.mods.get(mod_name)?.join(rel);
            return path.is_file().then(|| (path, mod_name.to_owned()));
        }

        let rel = to_path(name);
        if !safe_relative(&rel) {
            return None;
        }
        let stack = self.stack.borrow();
        let frame = stack.last()?;
        let candidates = [
            (frame.dir.join(&rel), frame.mod_name.clone()),
            (self.mods.get(&frame.mod_name)?.join(&rel), frame.mod_name.clone()),
            (self.lualib.join(&rel), "core".to_owned()),
        ];
        candidates.into_iter().find(|(p, _)| p.is_file())
    }

    fn require_file(&self, lua: &Lua, path: &Path, mod_name: &str) -> Result<Value> {
        if let Some(key) = self.loaded.borrow().get(path) {
            return Ok(lua.registry_value(key)?);
        }
        let mut src = crate::read(path)?;
        if src.starts_with(b"\xEF\xBB\xBF") {
            src.drain(..3);
        }
        let root = &self.mods[mod_name];
        let rel = path.strip_prefix(root).unwrap_or(path);
        let chunk_name = format!("@__{mod_name}__/{}", rel.display());

        self.stack.borrow_mut().push(Frame { mod_name: mod_name.to_owned(), dir: path.parent().unwrap().to_owned() });
        let result = lua.load(&src[..]).set_name(chunk_name).call::<MultiValue>(());
        self.stack.borrow_mut().pop();

        let value = result?.into_iter().next().unwrap_or(Value::Nil);
        let value = if value.is_nil() { Value::Boolean(true) } else { value };
        self.loaded.borrow_mut().insert(path.to_owned(), lua.create_registry_value(value.clone())?);
        Ok(value)
    }
}

fn safe_relative(p: &Path) -> bool {
    p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}
