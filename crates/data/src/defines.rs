//! Builds the `defines` table from the game's own `doc-html/runtime-api.json`.
//!
//! The JSON lists every define but not its numeric value; for enum-like defines the
//! `order` field is the value (e.g. `defines.direction.east == 4`), which is what data-stage
//! code relies on. A few scalar defines are filled in by hand.

use std::path::Path;

use mlua::{Lua, Table};
use serde::Deserialize;

use crate::{Error, Result};

#[derive(Debug, Deserialize)]
pub struct DefineNode {
    pub name: String,
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub values: Vec<DefineNode>,
    #[serde(default)]
    pub subkeys: Vec<DefineNode>,
}

#[derive(Debug, Deserialize)]
struct RuntimeApi {
    defines: Vec<DefineNode>,
}

pub fn read_defines(path: &Path) -> Result<Vec<DefineNode>> {
    let bytes = crate::read(path)?;
    let api: RuntimeApi =
        serde_json::from_slice(&bytes).map_err(|source| Error::Json { path: path.to_owned(), source })?;
    Ok(api.defines)
}

/// Top-level prototype category -> list of prototype `type`s, from `defines.prototypes`.
/// E.g. `"item" -> ["ammo", "armor", "item", "tool", ...]`.
pub fn prototype_categories(defines: &[DefineNode]) -> Vec<(String, Vec<String>)> {
    defines
        .iter()
        .find(|d| d.name == "prototypes")
        .map(|p| {
            p.subkeys.iter().map(|c| (c.name.clone(), c.values.iter().map(|v| v.name.clone()).collect())).collect()
        })
        .unwrap_or_default()
}

pub fn to_lua(lua: &Lua, defines: &[DefineNode]) -> Result<Table> {
    let table = lua.create_table()?;
    for node in defines {
        match node.name.as_str() {
            "default_icon_size" => table.set("default_icon_size", 64)?,
            _ => table.set(node.name.as_str(), node_to_lua(lua, node)?)?,
        }
    }
    Ok(table)
}

fn node_to_lua(lua: &Lua, node: &DefineNode) -> Result<Table> {
    let t = lua.create_table()?;
    for v in &node.values {
        t.set(v.name.as_str(), v.order)?;
    }
    for sub in &node.subkeys {
        t.set(sub.name.as_str(), node_to_lua(lua, sub)?)?;
    }
    Ok(t)
}
