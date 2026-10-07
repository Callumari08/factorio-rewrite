//! A plain-Rust copy of Lua prototype data (`data.raw`), detached from the Lua VM.

use std::collections::BTreeMap;

use mlua::Value;

/// Lua data converted to Rust. Tables whose keys are exactly `1..=n` become arrays;
/// all other tables become string-keyed maps (numeric keys are stringified).
/// Functions, userdata and threads are dropped.
#[derive(Clone, Debug, PartialEq)]
pub enum RawValue {
    Nil,
    Bool(bool),
    /// Lua 5.2 numbers are all doubles; integral values are stored here.
    Int(i64),
    Num(f64),
    Str(String),
    Array(Vec<RawValue>),
    Table(BTreeMap<String, RawValue>),
}

static NIL: RawValue = RawValue::Nil;

impl RawValue {
    /// Looks up `key` in a table. Returns `Nil` for anything missing.
    pub fn get(&self, key: &str) -> &RawValue {
        match self {
            RawValue::Table(t) => t.get(key).unwrap_or(&NIL),
            _ => &NIL,
        }
    }

    /// Indexes an array with a 0-based index.
    pub fn at(&self, i: usize) -> &RawValue {
        match self {
            RawValue::Array(a) => a.get(i).unwrap_or(&NIL),
            _ => &NIL,
        }
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, RawValue::Nil)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            RawValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            RawValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            RawValue::Int(i) => Some(*i as f64),
            RawValue::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            RawValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Array elements; an empty table counts as an empty array.
    pub fn as_array(&self) -> &[RawValue] {
        match self {
            RawValue::Array(a) => a,
            _ => &[],
        }
    }

    pub fn as_table(&self) -> Option<&BTreeMap<String, RawValue>> {
        match self {
            RawValue::Table(t) => Some(t),
            _ => None,
        }
    }

    /// Converts a Lua value. Cycles are cut (the repeated table becomes `Nil`).
    pub fn from_lua(value: &Value) -> RawValue {
        convert(value, &mut Vec::new())
    }
}

fn convert(value: &Value, stack: &mut Vec<usize>) -> RawValue {
    match value {
        Value::Nil => RawValue::Nil,
        Value::Boolean(b) => RawValue::Bool(*b),
        Value::Integer(i) => RawValue::Int(*i),
        Value::Number(n) => number(*n),
        Value::String(s) => RawValue::Str(s.to_string_lossy()),
        Value::Table(t) => {
            let ptr = t.to_pointer() as usize;
            if stack.contains(&ptr) {
                return RawValue::Nil;
            }
            stack.push(ptr);
            let entries: Vec<(Value, Value)> = t.pairs::<Value, Value>().flatten().collect();
            let out = if !entries.is_empty() && is_sequence(&entries) {
                let mut items: Vec<(i64, RawValue)> =
                    entries.iter().map(|(k, v)| (key_int(k).unwrap(), convert(v, stack))).collect();
                items.sort_by_key(|(k, _)| *k);
                RawValue::Array(items.into_iter().map(|(_, v)| v).collect())
            } else {
                let mut map = BTreeMap::new();
                for (k, v) in &entries {
                    let key = match k {
                        Value::String(s) => s.to_string_lossy(),
                        other => match key_int(other) {
                            Some(i) => i.to_string(),
                            None => continue,
                        },
                    };
                    let v = convert(v, stack);
                    if !v.is_nil() {
                        map.insert(key, v);
                    }
                }
                RawValue::Table(map)
            };
            stack.pop();
            out
        }
        _ => RawValue::Nil,
    }
}

fn number(n: f64) -> RawValue {
    if n.fract() == 0.0 && n.abs() < 9.0e15 { RawValue::Int(n as i64) } else { RawValue::Num(n) }
}

fn key_int(k: &Value) -> Option<i64> {
    match k {
        Value::Integer(i) => Some(*i),
        Value::Number(n) if n.fract() == 0.0 => Some(*n as i64),
        _ => None,
    }
}

fn is_sequence(entries: &[(Value, Value)]) -> bool {
    let n = entries.len() as i64;
    entries.iter().all(|(k, _)| key_int(k).is_some_and(|i| i >= 1 && i <= n))
}

impl RawValue {
    /// Converts back into a Lua value, e.g. to hand settings from one stage to the next.
    pub fn to_lua(&self, lua: &mlua::Lua) -> mlua::Result<Value> {
        Ok(match self {
            RawValue::Nil => Value::Nil,
            RawValue::Bool(b) => Value::Boolean(*b),
            RawValue::Int(i) => Value::Number(*i as f64),
            RawValue::Num(n) => Value::Number(*n),
            RawValue::Str(s) => Value::String(lua.create_string(s)?),
            RawValue::Array(a) => {
                let t = lua.create_table()?;
                for (i, v) in a.iter().enumerate() {
                    t.raw_set(i + 1, v.to_lua(lua)?)?;
                }
                Value::Table(t)
            }
            RawValue::Table(m) => {
                let t = lua.create_table()?;
                for (k, v) in m {
                    t.raw_set(k.as_str(), v.to_lua(lua)?)?;
                }
                Value::Table(t)
            }
        })
    }
}
