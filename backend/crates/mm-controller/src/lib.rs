//! Sandboxed Rhai controller host, script validation and built-in reference controllers.
//!
//! Script contract (also in `docs/CONTROLLER_CONVERSION_PROMPT.md`): `fn init(info)` and
//! `fn step(obs)`, state kept in the `this` object map, tunables declared as
//! `// @tunable name default min max max_step "description"`.

mod builtin;
mod controller;
mod tunables;
mod validate;

pub use builtin::{Idle, WallFollower};
pub use controller::{CompileError, Limits, RhaiController, ScriptError};
pub use tunables::parse_tunables;
pub use validate::{stub_observations, validate, TraceStep, ValidationOutcome};

/// Right-hand-rule wall follower as a Rhai script.
pub const WALL_FOLLOWER_RHAI: &str = include_str!("../scripts/wall_follower.rhai");
/// Wall follower plus victim identification and the blue-tile stop.
pub const VICTIM_CHECK_RHAI: &str = include_str!("../scripts/victim_check.rhai");
/// Copy-paste prompt that converts Arduino C++ / MicroPython firmware into a script.
pub const CONVERSION_PROMPT: &str = include_str!("../../../docs/CONTROLLER_CONVERSION_PROMPT.md");

#[cfg(test)]
mod tests;
