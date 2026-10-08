//! English (or other language) names and descriptions from the game's locale files
//! (`<mod>/locale/<lang>/*.cfg`), resolved the way Factorio resolves default names.

use std::collections::BTreeMap;

use crate::datastage::GameData;
use crate::raw::RawValue;

#[derive(Clone, Debug, Default)]
pub struct Locale {
    /// `section.key` -> text, e.g. `item-name.iron-plate` -> `Iron plate`.
    entries: BTreeMap<String, String>,
}

impl Locale {
    /// Loads `lang` (falling back to English) for every enabled mod, in load order.
    pub fn load(data: &GameData, lang: &str) -> Locale {
        let mut entries = BTreeMap::new();
        for l in ["en", lang] {
            for m in &data.mods {
                let dir = m.root.join("locale").join(l);
                let Ok(rd) = std::fs::read_dir(&dir) else { continue };
                let mut files: Vec<_> =
                    rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "cfg")).collect();
                files.sort();
                for f in files {
                    if let Ok(text) = std::fs::read_to_string(&f) {
                        parse_cfg(&text, &mut entries);
                    }
                }
            }
            if lang == "en" {
                break;
            }
        }
        Locale { entries }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// Resolves a `LocalisedString` value from prototype data.
    pub fn resolve(&self, v: &RawValue) -> Option<String> {
        match v {
            RawValue::Str(s) => Some(self.substitute(s)),
            RawValue::Array(a) => {
                let key = a.first()?.as_str()?;
                let params: Vec<String> = a[1..].iter().map(|p| self.resolve(p).unwrap_or_default()).collect();
                if key.is_empty() {
                    return Some(params.concat());
                }
                let mut text = self.get(key)?.to_owned();
                for (i, p) in params.iter().enumerate() {
                    text = text.replace(&format!("__{}__", i + 1), p);
                }
                Some(self.substitute(&text))
            }
            _ => None,
        }
    }

    /// Replaces `__ITEM__x__`, `__ENTITY__x__` and friends.
    fn substitute(&self, s: &str) -> String {
        let mut out = s.to_owned();
        for (tag, section) in
            [("ITEM", "item-name"), ("ENTITY", "entity-name"), ("FLUID", "fluid-name"), ("TILE", "tile-name")]
        {
            let open = format!("__{tag}__");
            while let Some(start) = out.find(&open) {
                let rest = &out[start + open.len()..];
                let Some(end) = rest.find("__") else { break };
                let name = &rest[..end];
                let text = self.get(&format!("{section}.{name}")).unwrap_or(name).to_owned();
                out.replace_range(start..start + open.len() + end + 2, &text);
            }
        }
        out
    }

    fn named(&self, proto: &RawValue, section: &str, name: &str) -> Option<String> {
        if let Some(s) = self.resolve(proto.get("localised_name")) {
            return Some(s);
        }
        self.get(&format!("{section}.{name}")).map(|s| self.substitute(s))
    }

    /// Display name of an item, falling back to its place result's entity name.
    pub fn item_name(&self, data: &GameData, name: &str) -> String {
        let proto = data.prototypes_in_category("item").find(|(_, n, _)| *n == name).map(|(_, _, p)| p);
        let p = proto.unwrap_or(&RawValue::Nil);
        self.named(p, "item-name", name)
            .or_else(|| {
                p.get("place_result").as_str().and_then(|e| self.get(&format!("entity-name.{e}")).map(str::to_owned))
            })
            .or_else(|| self.get(&format!("entity-name.{name}")).map(str::to_owned))
            .unwrap_or_else(|| prettify(name))
    }

    pub fn entity_name(&self, data: &GameData, name: &str) -> String {
        let proto = data.prototypes_in_category("entity").find(|(_, n, _)| *n == name).map(|(_, _, p)| p);
        self.named(proto.unwrap_or(&RawValue::Nil), "entity-name", name).unwrap_or_else(|| prettify(name))
    }

    pub fn fluid_name(&self, data: &GameData, name: &str) -> String {
        self.named(data.prototype("fluid", name), "fluid-name", name).unwrap_or_else(|| prettify(name))
    }

    /// Display name of a recipe, falling back to its single product's name.
    pub fn recipe_name(&self, data: &GameData, name: &str) -> String {
        let p = data.prototype("recipe", name);
        if let Some(s) = self.named(p, "recipe-name", name) {
            return s;
        }
        let main = p.get("main_product").as_str().map(str::to_owned).or_else(|| {
            let results = p.get("results").as_array();
            (results.len() == 1).then(|| results[0].get("name").as_str().map(str::to_owned)).flatten()
        });
        match main {
            Some(m) if !data.prototype("fluid", &m).is_nil() => self.fluid_name(data, &m),
            Some(m) => self.item_name(data, &m),
            None => prettify(name),
        }
    }

    /// Display name of a technology and whether a level number belongs after it. Levelled
    /// technologies (`mining-productivity-3`) share the name of their base (`mining-productivity`).
    pub fn technology_name(&self, data: &GameData, name: &str) -> (String, bool) {
        let p = data.prototype("technology", name);
        if let Some(s) = self.named(p, "technology-name", name) {
            return (s, false);
        }
        if let Some((base, level)) = name.rsplit_once('-')
            && level.parse::<u32>().is_ok()
            && let Some(s) = self.get(&format!("technology-name.{base}"))
        {
            return (self.substitute(s), true);
        }
        (prettify(name), false)
    }

    /// A technology's description, falling back to its base name for levelled ones.
    pub fn technology_description(&self, name: &str) -> Option<String> {
        self.description("technology", name).or_else(|| {
            let (base, level) = name.rsplit_once('-')?;
            level.parse::<u32>().ok()?;
            self.description("technology", base)
        })
    }

    pub fn item_group_name(&self, name: &str) -> String {
        self.get(&format!("item-group-name.{name}")).map(str::to_owned).unwrap_or_else(|| prettify(name))
    }

    pub fn description(&self, section: &str, name: &str) -> Option<String> {
        self.get(&format!("{section}-description.{name}")).map(|s| self.substitute(s))
    }
}

/// `iron-gear-wheel` -> `Iron gear wheel`, for names without a locale entry.
fn prettify(name: &str) -> String {
    let s = name.replace('-', " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s,
    }
}

fn parse_cfg(text: &str, out: &mut BTreeMap<String, String>) {
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.to_owned();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = if section.is_empty() { k.to_owned() } else { format!("{section}.{k}") };
            out.insert(key, v.replace("\\n", "\n"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_and_substitutes() {
        let mut m = BTreeMap::new();
        parse_cfg(
            "[item-name]\niron-plate=Iron plate\n[entity-name]\nstone-furnace=Stone furnace\n[x]\ny=Uses __ITEM__iron-plate__",
            &mut m,
        );
        let l = Locale { entries: m };
        assert_eq!(l.get("item-name.iron-plate"), Some("Iron plate"));
        assert_eq!(l.resolve(&RawValue::Array(vec![RawValue::Str("x.y".into())])).as_deref(), Some("Uses Iron plate"));
        assert_eq!(prettify("iron-gear-wheel"), "Iron gear wheel");
    }
}
