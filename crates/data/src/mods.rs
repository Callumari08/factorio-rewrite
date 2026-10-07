//! Mod discovery and Factorio's load order.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::install::{Config, FactorioInstall};
use crate::{Error, Result};

#[derive(Clone, Debug, Deserialize)]
pub struct ModInfo {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(skip)]
    pub root: PathBuf,
    /// Every other `info.json` field, e.g. `quality_required`, `space_travel_required`.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepKind {
    Required,
    Optional,
    HiddenOptional,
    Incompatible,
    /// Required, but does not affect load order (`~`).
    RequiredNoOrder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub kind: DepKind,
    pub name: String,
}

/// Parses a dependency string such as `"? base >= 2.0"` or `"! some-mod"`.
pub fn parse_dependency(s: &str) -> Dependency {
    let s = s.trim();
    let (kind, rest) = if let Some(r) = s.strip_prefix("(?)") {
        (DepKind::HiddenOptional, r)
    } else if let Some(r) = s.strip_prefix('?') {
        (DepKind::Optional, r)
    } else if let Some(r) = s.strip_prefix('!') {
        (DepKind::Incompatible, r)
    } else if let Some(r) = s.strip_prefix('~') {
        (DepKind::RequiredNoOrder, r)
    } else {
        (DepKind::Required, s)
    };
    let rest = rest.trim();
    let end = rest.find(['<', '>', '=']).unwrap_or(rest.len());
    Dependency { kind, name: rest[..end].trim().to_owned() }
}

impl ModInfo {
    pub fn read(root: &Path) -> Result<ModInfo> {
        let path = root.join("info.json");
        let bytes = crate::read(&path)?;
        let mut info: ModInfo = serde_json::from_slice(&bytes).map_err(|source| Error::Json { path, source })?;
        info.root = root.to_owned();
        Ok(info)
    }

    pub fn deps(&self) -> impl Iterator<Item = Dependency> + '_ {
        self.dependencies.iter().map(|d| parse_dependency(d))
    }
}

/// Finds every enabled mod and returns them in load order, with `core` first.
///
/// Enabled = `core`, `base`, and `config.mods`. Mods are looked up in the install's
/// `data/` dir (where Space Age and friends ship), then `config.mod_dirs`, then the user's
/// Factorio `mods/` folder. Only unpacked mods are supported for now.
pub fn resolve_mods(install: &FactorioInstall, config: &Config) -> Result<Vec<ModInfo>> {
    let mut search: Vec<PathBuf> = vec![install.data_dir.clone()];
    search.extend(config.mod_dirs.iter().cloned());
    if let Some(user) = FactorioInstall::user_dir() {
        search.push(user.join("mods"));
    }

    let mut wanted: Vec<String> = vec!["base".into()];
    wanted.extend(config.mods.iter().cloned());

    let mut enabled: BTreeMap<String, ModInfo> = BTreeMap::new();
    for name in &wanted {
        let root = search
            .iter()
            .map(|dir| dir.join(name))
            .find(|p| p.join("info.json").is_file())
            .ok_or_else(|| Error::Mod(format!("enabled mod '{name}' not found in {search:?}")))?;
        enabled.insert(name.clone(), ModInfo::read(&root)?);
    }

    let mut ordered = vec![ModInfo::read(&install.data_dir.join("core"))?];
    ordered.extend(load_order(enabled)?);
    Ok(ordered)
}

/// Factorio's ordering (see the game's `doc-html/auxiliary/data-lifecycle.html`): mods are
/// sorted by the depth of their dependency chain first, then by the natural sort order of
/// their internal names. Missing required dependencies are an error.
pub fn load_order(enabled: BTreeMap<String, ModInfo>) -> Result<Vec<ModInfo>> {
    let names: BTreeSet<String> = enabled.keys().cloned().collect();
    let mut after: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, info) in &enabled {
        let mut set = BTreeSet::new();
        for dep in info.deps() {
            let present = names.contains(&dep.name);
            match dep.kind {
                DepKind::Required | DepKind::RequiredNoOrder if !present && dep.name != "core" => {
                    return Err(Error::Mod(format!("{name} requires missing mod {}", dep.name)));
                }
                DepKind::Incompatible if present => {
                    return Err(Error::Mod(format!("{name} is incompatible with {}", dep.name)));
                }
                DepKind::Required | DepKind::Optional | DepKind::HiddenOptional if present => {
                    set.insert(dep.name);
                }
                _ => {}
            }
        }
        after.insert(name.clone(), set);
    }

    let mut depth: BTreeMap<String, usize> = BTreeMap::new();
    for name in &names {
        dependency_depth(name, &after, &mut depth, &mut BTreeSet::new())?;
    }

    let mut mods: Vec<ModInfo> = enabled.into_values().collect();
    mods.sort_by(|a, b| depth[&a.name].cmp(&depth[&b.name]).then_with(|| natural_cmp(&a.name, &b.name)));
    Ok(mods)
}

fn dependency_depth(
    name: &str,
    after: &BTreeMap<String, BTreeSet<String>>,
    memo: &mut BTreeMap<String, usize>,
    visiting: &mut BTreeSet<String>,
) -> Result<usize> {
    if let Some(d) = memo.get(name) {
        return Ok(*d);
    }
    if !visiting.insert(name.to_owned()) {
        return Err(Error::Mod(format!("dependency cycle involving {name}")));
    }
    let mut d = 0;
    for dep in &after[name] {
        d = d.max(dependency_depth(dep, after, memo, visiting)? + 1);
    }
    visiting.remove(name);
    memo.insert(name.to_owned(), d);
    Ok(d)
}

/// Natural string order: digit runs compare numerically (`mod2` < `mod10`), letters
/// case-insensitively, with an exact comparison as the final tie-break.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    natural_cmp_ci(a.as_bytes(), b.as_bytes()).then_with(|| a.cmp(b))
}

fn natural_cmp_ci(mut a: &[u8], mut b: &[u8]) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = a.iter().take_while(|c| c.is_ascii_digit()).count();
                let nb = b.iter().take_while(|c| c.is_ascii_digit()).count();
                let (da, db) = (trim_zeros(&a[..na]), trim_zeros(&b[..nb]));
                let ord = da.len().cmp(&db.len()).then_with(|| da.cmp(db));
                if ord != Ordering::Equal {
                    return ord;
                }
                a = &a[na..];
                b = &b[nb..];
            }
            (Some(x), Some(y)) => {
                let ord = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a = &a[1..];
                b = &b[1..];
            }
        }
    }
}

fn trim_zeros(s: &[u8]) -> &[u8] {
    let n = s.iter().take_while(|c| **c == b'0').count();
    &s[n..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str, deps: &[&str]) -> (String, ModInfo) {
        let info = ModInfo {
            name: name.into(),
            version: "1.0.0".into(),
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            root: PathBuf::new(),
            extra: BTreeMap::new(),
        };
        (name.into(), info)
    }

    #[test]
    fn parses_dependency_prefixes() {
        assert_eq!(parse_dependency("? quality >= 2.0").kind, DepKind::Optional);
        assert_eq!(parse_dependency("(?) foo").name, "foo");
        assert_eq!(parse_dependency("base >= 2.0.0").name, "base");
        assert_eq!(parse_dependency("~ x").kind, DepKind::RequiredNoOrder);
    }

    #[test]
    fn orders_like_factorio() {
        let mods = BTreeMap::from([
            m("base", &[]),
            m("space-age", &["base", "elevated-rails", "quality"]),
            m("quality", &["base"]),
            m("elevated-rails", &["base"]),
            m("aaa", &["? space-age"]),
        ]);
        let order: Vec<_> = load_order(mods).unwrap().into_iter().map(|m| m.name).collect();
        assert_eq!(order, ["base", "elevated-rails", "quality", "space-age", "aaa"]);
    }

    #[test]
    fn natural_sort() {
        let mods = BTreeMap::from([m("mod10", &[]), m("mod2", &[]), m("Mod3", &[])]);
        let order: Vec<_> = load_order(mods).unwrap().into_iter().map(|m| m.name).collect();
        assert_eq!(order, ["mod2", "Mod3", "mod10"]);
    }
}
