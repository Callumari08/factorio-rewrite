//! Compiles noise expressions into a flat program of operations.
//!
//! Like the game's compiler, names are resolved (locals and parameters, then
//! `property_expression_names`, then named expressions and functions, then built-ins),
//! functions are inlined, constants are folded, and identical operations are shared, so
//! each distinct sub-expression is evaluated once per point.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use super::parse::{Args, BinOp, Expr, parse};

pub type NodeId = u32;

/// A named noise expression or function (or an autoplace expression with its locals).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoiseDef {
    pub params: Vec<String>,
    pub expression: String,
    pub locals: BTreeMap<String, String>,
    pub local_functions: BTreeMap<String, NoiseDef>,
}

impl NoiseDef {
    pub fn expr(s: impl Into<String>) -> Self {
        NoiseDef { expression: s.into(), ..Default::default() }
    }
}

/// Everything expressions can refer to, from the game data and the map settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoiseInputs {
    pub expressions: BTreeMap<String, NoiseDef>,
    pub functions: BTreeMap<String, NoiseDef>,
    pub property_names: BTreeMap<String, String>,
    /// Extra variables such as `tile:grass-1:probability`.
    pub variables: BTreeMap<String, NoiseDef>,
}

/// Built-in constants: map settings, seeds, control sliders.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Constants {
    pub numbers: BTreeMap<String, f64>,
    pub points: BTreeMap<String, Vec<(f32, f32)>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NoiseParams {
    pub seed0: u32,
    pub seed1: u32,
    pub input_scale: f32,
    pub output_scale: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub octaves: u32,
    pub persistence: f32,
    pub octave_input_scale_multiplier: f32,
    pub octave_output_scale_multiplier: f32,
    pub octave_seed0_shift: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpotParams {
    pub density: NodeId,
    pub quantity: NodeId,
    pub radius: NodeId,
    pub favorability: NodeId,
    pub seed0: u32,
    pub seed1: u32,
    pub basement_value: f32,
    pub maximum_spot_basement_radius: f32,
    pub region_size: f32,
    pub skip_offset: u32,
    pub skip_span: u32,
    pub hard_region_target_quantity: bool,
    pub candidate_points: u32,
    pub spacing: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Const(f64),
    X,
    Y,
    Neg,
    BitNot,
    Bin(BinOp),
    /// `a ^ n` for a constant integer `n`.
    PowInt(i32),
    PowPrecise,
    Sqrt,
    Abs,
    Floor,
    Ceil,
    Sin,
    Cos,
    Log2,
    Atan2,
    Clamp,
    Min,
    Max,
    If,
    Ridge,
    Basis(NoiseParams),
    Multioctave(NoiseParams),
    /// Persistence is the third argument.
    VariablePersistence(NoiseParams),
    QuickMultioctave(NoiseParams),
    RandomPenalty {
        seed: u32,
        amplitude: f32,
    },
    /// 0: distance, 1: x offset, 2: y offset to the nearest point.
    NearestPoint {
        points: Arc<Vec<(f32, f32)>>,
        maximum_distance: f32,
        mode: u8,
    },
    ExpressionInRange {
        peak_multiplier: f32,
        peak_maximum: f32,
        from: Vec<f32>,
        to: Vec<f32>,
    },
    Spot(Box<SpotParams>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub op: Op,
    pub args: Vec<NodeId>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    pub nodes: Vec<Node>,
    intern: HashMap<String, NodeId>,
}

impl Program {
    pub fn constant(&self, id: NodeId) -> Option<f64> {
        match self.nodes[id as usize].op {
            Op::Const(v) => Some(v),
            _ => None,
        }
    }

    fn add(&mut self, op: Op, args: Vec<NodeId>) -> NodeId {
        // Fold operations whose arguments are all constants.
        let foldable = !matches!(
            op,
            Op::X | Op::Y | Op::Const(_) | Op::Spot(_) | Op::RandomPenalty { .. } | Op::NearestPoint { .. }
        ) && !args.is_empty()
            && args.iter().all(|a| self.constant(*a).is_some());
        if foldable {
            let vals: Vec<f32> = args.iter().map(|a| self.constant(*a).unwrap() as f32).collect();
            let v = super::eval::scalar(&op, &vals);
            return self.add(Op::Const(v as f64), Vec::new());
        }
        let key = format!("{op:?}|{args:?}");
        if let Some(id) = self.intern.get(&key) {
            return *id;
        }
        let id = self.nodes.len() as NodeId;
        self.nodes.push(Node { op, args });
        self.intern.insert(key, id);
        id
    }

    pub fn konst(&mut self, v: f64) -> NodeId {
        self.add(Op::Const(v), Vec::new())
    }
}

#[derive(Clone, Debug)]
enum Value {
    Node(NodeId),
    Str(String),
    Points(Arc<Vec<(f32, f32)>>),
}

/// A local scope: function parameters and a definition's local expressions/functions.
struct Scope {
    params: HashMap<String, Value>,
    locals: BTreeMap<String, String>,
    functions: BTreeMap<String, NoiseDef>,
    cache: std::cell::RefCell<HashMap<String, Value>>,
    parent: Option<Rc<Scope>>,
}

pub struct Compiler<'a> {
    inputs: &'a NoiseInputs,
    constants: &'a Constants,
    pub program: Program,
    globals: HashMap<String, Value>,
    parsed: HashMap<String, Arc<Expr>>,
    active: Vec<String>,
}

type R<T> = Result<T, String>;

impl<'a> Compiler<'a> {
    pub fn new(inputs: &'a NoiseInputs, constants: &'a Constants) -> Self {
        Compiler {
            inputs,
            constants,
            program: Program::default(),
            globals: HashMap::new(),
            parsed: HashMap::new(),
            active: Vec::new(),
        }
    }

    /// Compiles a top-level definition (an autoplace expression or a named expression).
    pub fn compile(&mut self, def: &NoiseDef) -> R<NodeId> {
        let scope = Rc::new(Scope {
            params: HashMap::new(),
            locals: def.locals.clone(),
            functions: def.local_functions.clone(),
            cache: Default::default(),
            parent: None,
        });
        let v = self.source(&def.expression, &scope)?;
        self.node(v)
    }

    /// Compiles a named variable such as `elevation`.
    pub fn compile_name(&mut self, name: &str) -> R<NodeId> {
        let scope = Rc::new(Scope {
            params: HashMap::new(),
            locals: BTreeMap::new(),
            functions: BTreeMap::new(),
            cache: Default::default(),
            parent: None,
        });
        let v = self.var(name, &scope)?;
        self.node(v)
    }

    fn node(&mut self, v: Value) -> R<NodeId> {
        match v {
            Value::Node(n) => Ok(n),
            Value::Str(s) => Ok(self.program.konst(super::basis::crc32(&s) as f64)),
            Value::Points(_) => Err("a point list cannot be used as a number".into()),
        }
    }

    fn source(&mut self, src: &str, scope: &Rc<Scope>) -> R<Value> {
        let expr = match self.parsed.get(src) {
            Some(e) => e.clone(),
            None => {
                let e = Arc::new(parse(src).map_err(|e| format!("{e} in '{src}'"))?);
                self.parsed.insert(src.to_owned(), e.clone());
                e
            }
        };
        self.expr(&expr, scope)
    }

    fn expr(&mut self, e: &Expr, scope: &Rc<Scope>) -> R<Value> {
        Ok(match e {
            Expr::Num(n) => Value::Node(self.program.konst(*n)),
            Expr::Str(s) => Value::Str(s.clone()),
            Expr::Var(name) => self.var(name, scope)?,
            Expr::Unary(op, a) => {
                let v = self.expr(a, scope)?;
                let a = self.node(v)?;
                match op {
                    '-' => Value::Node(self.program.add(Op::Neg, vec![a])),
                    '~' => Value::Node(self.program.add(Op::BitNot, vec![a])),
                    _ => Value::Node(a),
                }
            }
            Expr::Binary(op, a, b) => {
                let a = self.expr(a, scope)?;
                let a = self.node(a)?;
                let b = self.expr(b, scope)?;
                let b = self.node(b)?;
                Value::Node(self.binary(*op, a, b))
            }
            Expr::Call(name, args) => self.call(name, args, scope)?,
        })
    }

    fn binary(&mut self, op: BinOp, a: NodeId, b: NodeId) -> NodeId {
        if op == BinOp::Pow
            && let Some(p) = self.program.constant(b)
        {
            // The game's rewrites: x^0.5 is sqrt, integer powers are exact.
            if p == 0.5 {
                return self.program.add(Op::Sqrt, vec![a]);
            }
            if p.fract() == 0.0 && p.abs() <= 64.0 {
                return self.program.add(Op::PowInt(p as i32), vec![a]);
            }
        }
        self.program.add(Op::Bin(op), vec![a, b])
    }

    fn var(&mut self, name: &str, scope: &Rc<Scope>) -> R<Value> {
        // 1. Parameters and local expressions, innermost first.
        let mut s = Some(scope.clone());
        while let Some(sc) = s {
            if let Some(v) = sc.params.get(name) {
                return Ok(v.clone());
            }
            if let Some(src) = sc.locals.get(name) {
                if let Some(v) = sc.cache.borrow().get(name) {
                    return Ok(v.clone());
                }
                let key = format!("local:{name}:{:p}", Rc::as_ptr(&sc));
                self.enter(&key)?;
                let v = self.source(&src.clone(), &sc);
                self.active.pop();
                let v = v?;
                sc.cache.borrow_mut().insert(name.to_owned(), v.clone());
                return Ok(v);
            }
            s = sc.parent.clone();
        }
        // 2. Property overrides, 3. named expressions and extra variables.
        let inputs = self.inputs;
        let global = inputs
            .property_names
            .get(name)
            .and_then(|n| inputs.expressions.get(n).map(|d| (n.as_str(), d)))
            .or_else(|| inputs.expressions.get(name).map(|d| (name, d)))
            .or_else(|| inputs.variables.get(name).map(|d| (name, d)));
        if let Some((key, def)) = global {
            if let Some(v) = self.globals.get(key) {
                return Ok(v.clone());
            }
            self.enter(key)?;
            let root = Rc::new(Scope {
                params: HashMap::new(),
                locals: def.locals.clone(),
                functions: def.local_functions.clone(),
                cache: Default::default(),
                parent: None,
            });
            let v = self.source(&def.expression, &root);
            self.active.pop();
            let v = v?;
            self.globals.insert(key.to_owned(), v.clone());
            return Ok(v);
        }
        // 4. Built-ins.
        let k = |p: &mut Program, v: f64| Value::Node(p.konst(v));
        Ok(match name {
            "x" => Value::Node(self.program.add(Op::X, Vec::new())),
            "y" => Value::Node(self.program.add(Op::Y, Vec::new())),
            "pi" => k(&mut self.program, std::f64::consts::PI),
            "e" => k(&mut self.program, std::f64::consts::E),
            "inf" => k(&mut self.program, f64::INFINITY),
            "true" => k(&mut self.program, 1.0),
            "false" => k(&mut self.program, 0.0),
            _ => {
                if let Some(v) = self.constants.numbers.get(name) {
                    k(&mut self.program, *v)
                } else if let Some(p) = self.constants.points.get(name) {
                    Value::Points(Arc::new(p.clone()))
                } else {
                    return Err(format!("unknown variable '{name}'"));
                }
            }
        })
    }

    fn enter(&mut self, key: &str) -> R<()> {
        if self.active.iter().any(|a| a == key) {
            return Err(format!("recursive definition of '{key}'"));
        }
        self.active.push(key.to_owned());
        Ok(())
    }

    fn find_function(&self, name: &str, scope: &Rc<Scope>) -> Option<(NoiseDef, Option<Rc<Scope>>)> {
        let mut s = Some(scope.clone());
        while let Some(sc) = s {
            if let Some(f) = sc.functions.get(name) {
                return Some((f.clone(), Some(sc.clone())));
            }
            s = sc.parent.clone();
        }
        self.inputs.functions.get(name).map(|f| (f.clone(), None))
    }

    fn call(&mut self, name: &str, args: &Args, scope: &Rc<Scope>) -> R<Value> {
        if let Some((def, parent)) = self.find_function(name, scope) {
            let mut params = HashMap::new();
            match args {
                Args::Positional(list) => {
                    if list.len() > def.params.len() {
                        return Err(format!("too many arguments to {name}"));
                    }
                    for (p, a) in def.params.iter().zip(list) {
                        params.insert(p.clone(), self.expr(a, scope)?);
                    }
                }
                Args::Named(list) => {
                    for (p, a) in list {
                        if def.params.contains(p) {
                            params.insert(p.clone(), self.expr(a, scope)?);
                        }
                    }
                }
            }
            if let Some(missing) = def.params.iter().find(|p| !params.contains_key(*p)) {
                return Err(format!("missing argument '{missing}' in call to {name}"));
            }
            let inner = Rc::new(Scope {
                params,
                locals: def.locals.clone(),
                functions: def.local_functions.clone(),
                cache: Default::default(),
                parent,
            });
            let key = format!("fn:{name}:{:p}", Rc::as_ptr(&inner));
            self.enter(&key)?;
            let v = self.source(&def.expression, &inner);
            self.active.pop();
            return v;
        }
        self.builtin(name, args, scope)
    }

    fn builtin(&mut self, name: &str, args: &Args, scope: &Rc<Scope>) -> R<Value> {
        // Positional parameter order of each built-in, from the game's documentation.
        let order: &[&str] = match name {
            "abs" | "ceil" | "floor" | "sqrt" | "sin" | "cos" | "log2" | "var" | "noise_layer_id" => &["value"],
            "atan2" => &["y", "x"],
            "clamp" | "ridge" => &["value", "min", "max"],
            "if" => &["condition", "true_branch", "false_branch"],
            "pow" | "pow_precise" => &["value", "exponent"],
            "basis_noise" => &["x", "y", "seed0", "seed1", "input_scale", "output_scale", "offset_x", "offset_y"],
            "multioctave_noise" | "variable_persistence_multioctave_noise" => &[
                "x",
                "y",
                "persistence",
                "seed0",
                "seed1",
                "octaves",
                "input_scale",
                "output_scale",
                "offset_x",
                "offset_y",
            ],
            "quick_multioctave_noise" => &[
                "x",
                "y",
                "seed0",
                "seed1",
                "octaves",
                "input_scale",
                "output_scale",
                "offset_x",
                "offset_y",
                "octave_input_scale_multiplier",
                "octave_output_scale_multiplier",
                "octave_seed0_shift",
            ],
            "random_penalty" => &["x", "y", "source", "seed", "amplitude"],
            "distance_from_nearest_point" => &["x", "y", "points", "maximum_distance"],
            "distance_from_nearest_point_x" | "distance_from_nearest_point_y" => &["x", "y", "points"],
            "min" | "max" | "expression_in_range" => &[],
            "spot_noise" => &[
                "x",
                "y",
                "density_expression",
                "spot_quantity_expression",
                "spot_radius_expression",
                "spot_favorability_expression",
                "seed0",
                "seed1",
                "basement_value",
                "maximum_spot_basement_radius",
                "region_size",
                "skip_offset",
                "skip_span",
                "hard_region_target_quantity",
                "candidate_point_count",
                "candidate_spot_count",
                "suggested_minimum_candidate_point_spacing",
            ],
            _ => return Err(format!("unknown function '{name}'")),
        };

        // Variadic positional built-ins.
        if order.is_empty() {
            let Args::Positional(list) = args else { return Err(format!("{name} takes positional arguments")) };
            let mut ids = Vec::new();
            for a in list {
                let v = self.expr(a, scope)?;
                ids.push(self.node(v)?);
            }
            return Ok(Value::Node(match name {
                "min" => self.program.add(Op::Min, ids),
                "max" => self.program.add(Op::Max, ids),
                _ => {
                    // peak_multiplier, peak_maximum, then n expressions, n froms, n tos.
                    if ids.len() < 5 || (ids.len() - 2) % 3 != 0 {
                        return Err("expression_in_range needs 2 + 3n arguments".into());
                    }
                    let n = (ids.len() - 2) / 3;
                    let c = |c: &Self, i: usize| c.program.constant(ids[i]).map(|v| v as f32);
                    let peak_multiplier = c(self, 0).ok_or("expression_in_range: constant peak_multiplier")?;
                    let peak_maximum = c(self, 1).ok_or("expression_in_range: constant peak_maximum")?;
                    let from: Option<Vec<f32>> = (0..n).map(|i| c(self, 2 + n + i)).collect();
                    let to: Option<Vec<f32>> = (0..n).map(|i| c(self, 2 + 2 * n + i)).collect();
                    let (Some(from), Some(to)) = (from, to) else {
                        return Err("expression_in_range: ranges must be constant".into());
                    };
                    let exprs = ids[2..2 + n].to_vec();
                    self.program.add(Op::ExpressionInRange { peak_multiplier, peak_maximum, from, to }, exprs)
                }
            }));
        }

        let mut vals: HashMap<&str, Value> = HashMap::new();
        match args {
            Args::Positional(list) => {
                if list.len() > order.len() {
                    return Err(format!("too many arguments to {name}"));
                }
                for (p, a) in order.iter().zip(list) {
                    vals.insert(p, self.expr(a, scope)?);
                }
            }
            Args::Named(list) => {
                for (p, a) in list {
                    let Some(key) = order.iter().find(|o| **o == p.as_str()) else {
                        return Err(format!("unknown argument '{p}' to {name}"));
                    };
                    // Expression arguments of spot noise are compiled lazily below.
                    vals.insert(key, self.expr(a, scope)?);
                }
            }
        }
        let mut node = |c: &mut Self, k: &str| -> R<NodeId> {
            let v = vals.get(k).cloned().ok_or_else(|| format!("missing argument '{k}' to {name}"))?;
            c.node(v)
        };
        let constant = |c: &mut Self, vals: &HashMap<&str, Value>, k: &str, default: Option<f64>| -> R<f64> {
            match vals.get(k) {
                Some(Value::Str(s)) => Ok(super::basis::crc32(s) as f64),
                Some(v) => {
                    let id = c.node(v.clone())?;
                    c.program.constant(id).ok_or_else(|| format!("argument '{k}' to {name} must be constant"))
                }
                None => default.ok_or_else(|| format!("missing argument '{k}' to {name}")),
            }
        };
        let seed = |v: f64| v as i64 as u32;

        let op1 = |c: &mut Self, op: Op, node: &mut dyn FnMut(&mut Self, &str) -> R<NodeId>| -> R<Value> {
            let a = node(c, "value")?;
            Ok(Value::Node(c.program.add(op, vec![a])))
        };
        Ok(match name {
            "var" => match vals.get("value") {
                Some(Value::Str(s)) => self.var(&s.clone(), scope)?,
                _ => return Err("var() takes a string".into()),
            },
            "noise_layer_id" => match vals.get("value") {
                Some(Value::Str(s)) => Value::Node(self.program.konst(super::basis::crc32(s) as f64)),
                Some(v) => v.clone(),
                None => return Err("noise_layer_id() takes a string".into()),
            },
            "abs" => op1(self, Op::Abs, &mut node)?,
            "ceil" => op1(self, Op::Ceil, &mut node)?,
            "floor" => op1(self, Op::Floor, &mut node)?,
            "sqrt" => op1(self, Op::Sqrt, &mut node)?,
            "sin" => op1(self, Op::Sin, &mut node)?,
            "cos" => op1(self, Op::Cos, &mut node)?,
            "log2" => op1(self, Op::Log2, &mut node)?,
            "atan2" => {
                let (y, x) = (node(self, "y")?, node(self, "x")?);
                Value::Node(self.program.add(Op::Atan2, vec![y, x]))
            }
            "clamp" | "ridge" => {
                let ids = vec![node(self, "value")?, node(self, "min")?, node(self, "max")?];
                Value::Node(self.program.add(if name == "clamp" { Op::Clamp } else { Op::Ridge }, ids))
            }
            "if" => {
                let ids = vec![node(self, "condition")?, node(self, "true_branch")?, node(self, "false_branch")?];
                Value::Node(self.program.add(Op::If, ids))
            }
            "pow" => {
                let (a, b) = (node(self, "value")?, node(self, "exponent")?);
                Value::Node(self.binary(BinOp::Pow, a, b))
            }
            "pow_precise" => {
                let ids = vec![node(self, "value")?, node(self, "exponent")?];
                Value::Node(self.program.add(Op::PowPrecise, ids))
            }
            "basis_noise"
            | "multioctave_noise"
            | "variable_persistence_multioctave_noise"
            | "quick_multioctave_noise" => {
                let quick = name == "quick_multioctave_noise";
                let variable = name == "variable_persistence_multioctave_noise";
                let params = NoiseParams {
                    seed0: seed(constant(self, &vals, "seed0", None)?),
                    seed1: seed(constant(self, &vals, "seed1", None)?),
                    input_scale: constant(self, &vals, "input_scale", Some(1.0))? as f32,
                    output_scale: constant(self, &vals, "output_scale", Some(1.0))? as f32,
                    offset_x: constant(self, &vals, "offset_x", Some(0.0))? as f32,
                    offset_y: constant(self, &vals, "offset_y", Some(0.0))? as f32,
                    octaves: if name == "basis_noise" { 1 } else { constant(self, &vals, "octaves", None)? as u32 },
                    persistence: if name == "multioctave_noise" {
                        constant(self, &vals, "persistence", None)? as f32
                    } else {
                        0.0
                    },
                    octave_input_scale_multiplier: if quick {
                        constant(self, &vals, "octave_input_scale_multiplier", Some(0.5))? as f32
                    } else {
                        0.0
                    },
                    octave_output_scale_multiplier: if quick {
                        constant(self, &vals, "octave_output_scale_multiplier", Some(2.0))? as f32
                    } else {
                        0.0
                    },
                    octave_seed0_shift: if quick {
                        constant(self, &vals, "octave_seed0_shift", Some(1.0))? as u32
                    } else {
                        0
                    },
                };
                let mut ids = vec![node(self, "x")?, node(self, "y")?];
                if variable {
                    ids.push(node(self, "persistence")?);
                }
                let op = match name {
                    "basis_noise" => Op::Basis(params),
                    "multioctave_noise" => Op::Multioctave(params),
                    "variable_persistence_multioctave_noise" => Op::VariablePersistence(params),
                    _ => Op::QuickMultioctave(params),
                };
                Value::Node(self.program.add(op, ids))
            }
            "random_penalty" => {
                let op = Op::RandomPenalty {
                    seed: seed(constant(self, &vals, "seed", Some(1.0))?),
                    amplitude: constant(self, &vals, "amplitude", Some(1.0))? as f32,
                };
                let ids = vec![node(self, "x")?, node(self, "y")?, node(self, "source")?];
                Value::Node(self.program.add(op, ids))
            }
            "distance_from_nearest_point" | "distance_from_nearest_point_x" | "distance_from_nearest_point_y" => {
                let Some(Value::Points(points)) = vals.get("points").cloned() else {
                    return Err(format!("{name} needs a point list"));
                };
                let mode = match name {
                    "distance_from_nearest_point" => 0,
                    "distance_from_nearest_point_x" => 1,
                    _ => 2,
                };
                let maximum_distance = constant(self, &vals, "maximum_distance", Some(f64::INFINITY))? as f32;
                let ids = vec![node(self, "x")?, node(self, "y")?];
                Value::Node(self.program.add(Op::NearestPoint { points, maximum_distance, mode }, ids))
            }
            "spot_noise" => {
                let skip_span = constant(self, &vals, "skip_span", Some(1.0))?.max(1.0) as u32;
                let candidate_points =
                    match (vals.contains_key("candidate_point_count"), vals.contains_key("candidate_spot_count")) {
                        (true, _) => constant(self, &vals, "candidate_point_count", None)? as u32,
                        (false, true) => constant(self, &vals, "candidate_spot_count", None)? as u32 * skip_span,
                        _ => 256,
                    };
                let p = SpotParams {
                    density: node(self, "density_expression")?,
                    quantity: node(self, "spot_quantity_expression")?,
                    radius: node(self, "spot_radius_expression")?,
                    favorability: node(self, "spot_favorability_expression")?,
                    seed0: seed(constant(self, &vals, "seed0", None)?),
                    seed1: seed(constant(self, &vals, "seed1", None)?),
                    basement_value: constant(self, &vals, "basement_value", None)? as f32,
                    maximum_spot_basement_radius: constant(self, &vals, "maximum_spot_basement_radius", None)? as f32,
                    region_size: constant(self, &vals, "region_size", Some(512.0))? as f32,
                    skip_offset: constant(self, &vals, "skip_offset", Some(0.0))? as u32,
                    skip_span,
                    hard_region_target_quantity: constant(self, &vals, "hard_region_target_quantity", Some(1.0))? > 0.0,
                    candidate_points,
                    spacing: constant(self, &vals, "suggested_minimum_candidate_point_spacing", Some(0.0))? as f32,
                };
                let ids = vec![node(self, "x")?, node(self, "y")?];
                Value::Node(self.program.add(Op::Spot(Box::new(p)), ids))
            }
            _ => unreachable!(),
        })
    }
}
