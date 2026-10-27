//! The Rhai host: a sandboxed engine, one persistent `this` object map, strict command parsing.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use mm_core::{
    Commands, Controller, Observations, RobotInfo, TunableParam, VictimReport, VictimStatus,
};
use rhai::packages::{
    ArithmeticPackage, BasicArrayPackage, BasicIteratorPackage, BasicMapPackage, BasicMathPackage,
    BasicStringPackage, CorePackage, LogicPackage, MoreStringPackage, Package,
};
use rhai::{CallFnOptions, Dynamic, Engine, EvalAltResult, Map, ParseError, Position, Scope, AST};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::tunables::parse_tunables;

/// A problem in a script, with a source position when Rhai knows one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScriptError {
    pub line: Option<u32>,
    pub col: Option<u32>,
    pub message: String,
}

impl ScriptError {
    fn plain(message: impl Into<String>) -> Self {
        Self {
            line: None,
            col: None,
            message: message.into(),
        }
    }

    fn at(pos: Position, message: impl Into<String>) -> Self {
        Self {
            line: pos.line().map(|l| l as u32),
            col: pos.position().map(|c| c as u32),
            message: message.into(),
        }
    }

    fn from_runtime(e: &EvalAltResult) -> Self {
        let mut e = e;
        while let EvalAltResult::ErrorInFunctionCall(_, _, inner, _) = e {
            e = inner.as_ref();
        }
        let mut message = if matches!(e, EvalAltResult::ErrorTerminated(..)) {
            "fuel exhausted: the script ran too many operations in one step (infinite loop?)"
                .to_string()
        } else {
            e.to_string()
        };
        if let Some(i) = message.rfind(" (line ") {
            message.truncate(i);
        }
        Self::at(e.position(), message)
    }
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.col) {
            (Some(l), Some(c)) => write!(f, "line {l}, col {c}: {}", self.message),
            (Some(l), None) => write!(f, "line {l}: {}", self.message),
            _ => write!(f, "{}", self.message),
        }
    }
}

/// Script failed to compile or declared bad tunables.
#[derive(Debug, Clone, PartialEq)]
pub struct CompileError {
    pub errors: Vec<ScriptError>,
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.errors.iter().map(|e| e.to_string()).collect();
        write!(f, "{}", parts.join("; "))
    }
}

impl std::error::Error for CompileError {}

/// Sandbox budgets. `max_operations` is the fuel per `init`/`step` call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Limits {
    pub max_operations: u64,
    pub max_call_depth: usize,
    pub max_expr_depth: usize,
    pub max_string_len: usize,
    pub max_array_len: usize,
    pub max_map_len: usize,
    /// Log lines kept per step; extra `print`/`debug` output is dropped.
    pub max_log_lines: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_operations: 200_000,
            max_call_depth: 32,
            max_expr_depth: 64,
            max_string_len: 4096,
            max_array_len: 1024,
            max_map_len: 256,
            max_log_lines: 64,
        }
    }
}

type Shared<T> = Arc<Mutex<T>>;

fn lock<T>(m: &Shared<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

const COMMAND_KEYS: &str = "wheel_v, left, right, id_light, deploy_kits, report, end_run, note";

/// A user script running in a Rhai engine with no I/O, no clock and a fuel budget.
pub struct RhaiController {
    engine: Engine,
    ast: AST,
    scope: Scope<'static>,
    this: Dynamic,
    tunables: Vec<TunableParam>,
    params: Shared<Dynamic>,
    log: Shared<Vec<String>>,
    has_init: bool,
}

impl RhaiController {
    pub fn new(code: &str, limits: Limits) -> Result<Self, CompileError> {
        let tunables = parse_tunables(code).map_err(|errors| CompileError { errors })?;
        let params: Shared<Dynamic> = Arc::new(Mutex::new(Dynamic::from_map(Map::new())));
        let log: Shared<Vec<String>> = Arc::new(Mutex::new(Vec::new()));
        let engine = build_engine(&limits, &params, &log);
        let ast = engine.compile(code).map_err(|e: ParseError| CompileError {
            errors: vec![ScriptError::at(e.position(), e.err_type().to_string())],
        })?;
        let mut has_init = false;
        let mut has_step = false;
        for f in ast.iter_functions() {
            match (f.name, f.params.len()) {
                ("init", 1) => has_init = true,
                ("step", 1) => has_step = true,
                _ => {}
            }
        }
        if !has_step {
            return Err(CompileError {
                errors: vec![ScriptError::plain(
                    "script must define `fn step(obs)` taking exactly one argument",
                )],
            });
        }
        Ok(Self {
            engine,
            ast,
            scope: Scope::new(),
            this: Dynamic::from_map(Map::new()),
            tunables,
            params,
            log,
            has_init,
        })
    }

    /// The `@tunable` declarations found in the script.
    pub fn tunables(&self) -> &[TunableParam] {
        &self.tunables
    }

    /// Drain the `print`/`debug` output captured since the last call.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.log))
    }

    /// `init` with a structured error.
    pub fn run_init(&mut self, info: &RobotInfo) -> Result<(), ScriptError> {
        lock(&self.log).clear();
        self.this = Dynamic::from_map(Map::new());
        let mut resolved: BTreeMap<String, f64> = self
            .tunables
            .iter()
            .map(|t| {
                (
                    t.path.trim_start_matches("controller.").to_string(),
                    t.default,
                )
            })
            .collect();
        for (k, v) in &info.params {
            resolved.insert(k.trim_start_matches("controller.").to_string(), *v);
        }
        let mut pmap = Map::new();
        for (k, v) in &resolved {
            pmap.insert(k.as_str().into(), Dynamic::from_float(*v));
        }
        *lock(&self.params) = Dynamic::from_map(pmap.clone());
        if !self.has_init {
            return Ok(());
        }
        let mut m = Map::new();
        m.insert("tile_m".into(), Dynamic::from_float(info.tile_m));
        m.insert("dt".into(), Dynamic::from_float(info.controller_dt_s));
        m.insert("params".into(), Dynamic::from_map(pmap));
        let robot = rhai::serde::to_dynamic(&info.robot)
            .map_err(|e| ScriptError::plain(format!("cannot expose robot config: {e}")))?;
        m.insert("robot".into(), robot);
        self.call("init", Dynamic::from_map(m)).map(|_| ())
    }

    /// `step` with a structured error.
    pub fn run_step(&mut self, obs: &Observations) -> Result<Commands, ScriptError> {
        lock(&self.log).clear();
        let arg = rhai::serde::to_dynamic(obs)
            .map_err(|e| ScriptError::plain(format!("cannot expose observations: {e}")))?;
        let r = self.call("step", arg)?;
        parse_commands(r).map_err(ScriptError::plain)
    }

    fn call(&mut self, name: &str, arg: Dynamic) -> Result<Dynamic, ScriptError> {
        let opts = CallFnOptions::new()
            .eval_ast(false)
            .bind_this_ptr(&mut self.this);
        self.engine
            .call_fn_with_options::<Dynamic>(opts, &mut self.scope, &self.ast, name, (arg,))
            .map_err(|e| ScriptError::from_runtime(&e))
    }
}

impl Controller for RhaiController {
    fn init(&mut self, info: &RobotInfo) -> Result<(), String> {
        self.run_init(info).map_err(|e| format!("init: {e}"))
    }

    fn step(&mut self, obs: &Observations) -> Result<Commands, String> {
        self.run_step(obs).map_err(|e| format!("step: {e}"))
    }
}

fn build_engine(limits: &Limits, params: &Shared<Dynamic>, log: &Shared<Vec<String>>) -> Engine {
    let mut engine = Engine::new_raw();
    CorePackage::new().register_into_engine(&mut engine);
    ArithmeticPackage::new().register_into_engine(&mut engine);
    LogicPackage::new().register_into_engine(&mut engine);
    BasicMathPackage::new().register_into_engine(&mut engine);
    BasicIteratorPackage::new().register_into_engine(&mut engine);
    BasicStringPackage::new().register_into_engine(&mut engine);
    MoreStringPackage::new().register_into_engine(&mut engine);
    BasicArrayPackage::new().register_into_engine(&mut engine);
    BasicMapPackage::new().register_into_engine(&mut engine);

    engine.set_max_operations(limits.max_operations);
    engine.set_max_call_levels(limits.max_call_depth);
    engine.set_max_expr_depths(limits.max_expr_depth, limits.max_expr_depth / 2);
    engine.set_max_string_size(limits.max_string_len);
    engine.set_max_array_size(limits.max_array_len);
    engine.set_max_map_size(limits.max_map_len);
    let fuel = limits.max_operations;
    engine.on_progress(move |ops| {
        if ops >= fuel {
            Some("fuel exhausted: the script ran too many operations in one step".into())
        } else {
            None
        }
    });

    let cap = limits.max_log_lines;
    let print_log = Arc::clone(log);
    engine.on_print(move |s| push_log(&print_log, cap, s.to_string()));
    let debug_log = Arc::clone(log);
    engine.on_debug(move |s, _src, pos| {
        let at = match pos.line() {
            Some(l) => format!("[line {l}] "),
            None => String::new(),
        };
        push_log(&debug_log, cap, format!("{at}{s}"));
    });

    let p = Arc::clone(params);
    #[allow(deprecated)] // on_var is marked volatile, not removed
    engine.on_var(move |name, _idx, _ctx| {
        if name == "PARAMS" {
            Ok(Some(lock(&p).clone()))
        } else {
            Ok(None)
        }
    });
    engine
}

fn push_log(log: &Shared<Vec<String>>, cap: usize, mut line: String) {
    let mut g = lock(log);
    if g.len() < cap {
        if line.chars().count() > 256 {
            line = line.chars().take(256).collect();
        }
        g.push(line);
    }
}

fn as_f64(d: &Dynamic) -> Option<f64> {
    d.as_float()
        .ok()
        .or_else(|| d.as_int().ok().map(|i| i as f64))
}

fn finite(key: &str, d: &Dynamic) -> Result<f64, String> {
    match as_f64(d) {
        Some(v) if v.is_finite() => Ok(v),
        Some(_) => Err(format!("`{key}` must be finite (got NaN or infinity)")),
        None => Err(format!("`{key}` must be a number, got {}", d.type_name())),
    }
}

fn as_bool(key: &str, d: &Dynamic) -> Result<bool, String> {
    d.as_bool()
        .map_err(|_| format!("`{key}` must be a bool, got {}", d.type_name()))
}

fn parse_commands(r: Dynamic) -> Result<Commands, String> {
    if r.is_unit() {
        return Ok(Commands::default());
    }
    let type_name = r.type_name().to_string();
    let map = r.try_cast::<Map>().ok_or_else(|| {
        format!("step must return an object map `#{{ ... }}` or (), got {type_name}")
    })?;
    let mut c = Commands::default();
    let (mut wheel, mut left, mut right) = (None, None, None);
    let mut unknown = Vec::new();
    for (k, v) in map {
        match k.as_str() {
            "wheel_v" => {
                let arr = v
                    .into_array()
                    .map_err(|_| "`wheel_v` must be an array of 4 numbers".to_string())?;
                if arr.len() != 4 {
                    return Err(format!("`wheel_v` needs 4 numbers, got {}", arr.len()));
                }
                let mut w = [0.0; 4];
                for (slot, d) in w.iter_mut().zip(&arr) {
                    *slot = finite("wheel_v", d)?;
                }
                wheel = Some(w);
            }
            "left" => left = Some(finite("left", &v)?),
            "right" => right = Some(finite("right", &v)?),
            "id_light" => c.id_light = as_bool("id_light", &v)?,
            "end_run" => c.end_run = as_bool("end_run", &v)?,
            "deploy_kits" => {
                let n = v
                    .as_int()
                    .map_err(|_| "`deploy_kits` must be an integer".to_string())?;
                c.deploy_kits = u32::try_from(n)
                    .map_err(|_| format!("`deploy_kits` must be 0 or more, got {n}"))?;
            }
            "report" => {
                if !v.is_unit() {
                    let s = v
                        .into_string()
                        .map_err(|_| "`report` must be a string or ()".to_string())?;
                    let status = match s.as_str() {
                        "harmed" => VictimStatus::Harmed,
                        "stable" => VictimStatus::Stable,
                        "unharmed" => VictimStatus::Unharmed,
                        other => {
                            return Err(format!(
                                "`report` must be \"harmed\", \"stable\", \"unharmed\" or (), got \"{other}\""
                            ))
                        }
                    };
                    c.report = Some(VictimReport { status });
                }
            }
            "note" => {
                if !v.is_unit() {
                    let s = v
                        .into_string()
                        .map_err(|_| "`note` must be a string or ()".to_string())?;
                    c.note = Some(s);
                }
            }
            other => unknown.push(other.to_string()),
        }
    }
    if !unknown.is_empty() {
        return Err(format!(
            "unknown command key(s): {} (allowed: {COMMAND_KEYS})",
            unknown.join(", ")
        ));
    }
    match (wheel, left, right) {
        (Some(w), None, None) => c.wheel_v = w,
        (Some(_), _, _) => return Err("use either `wheel_v` or `left`/`right`, not both".into()),
        (None, l, r) => {
            let (l, r) = (l.unwrap_or(0.0), r.unwrap_or(0.0));
            c.wheel_v = [l, r, l, r];
        }
    }
    Ok(c)
}
