//! Run configuration and lifecycle state.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RngMode {
    /// Fresh OS-entropy seed per run.
    Trng,
    /// Caller-supplied seed: identical maze instance and noise realization.
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RngSpec {
    pub mode: RngMode,
    /// Required for `fixed`, ignored for `trng`.
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SimTiming {
    pub physics_dt_s: f64,
    pub controller_dt_s: f64,
    pub frame_dt_s: f64,
}

impl Default for SimTiming {
    fn default() -> Self {
        SimTiming {
            physics_dt_s: 0.001,
            controller_dt_s: 0.01,
            frame_dt_s: 1.0 / 25.0,
        }
    }
}

/// Everything needed to run a simulation, resolved (no ids).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunSettings {
    pub seed: u64,
    pub rng_mode: RngMode,
    pub timing: SimTiming,
    /// Overrides the rules' run duration when set.
    pub max_duration_s: Option<f64>,
    pub lop_enabled: bool,
}

/// `POST /api/runs` request body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CreateRun {
    pub robot_id: String,
    pub maze_id: String,
    pub controller_id: String,
    pub rng: RngSpec,
    /// Preset id (`clean`, `realistic`, `chaos`) or a stored custom profile id.
    pub error_profile_id: String,
    /// Live playback speed multiplier; 0 = as fast as possible (headless).
    pub speed: f64,
    pub max_duration_s: Option<f64>,
    #[serde(default = "default_true")]
    pub lop_enabled: bool,
    /// Controller tunable overrides, `name -> value`.
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, f64>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Queued,
    Running,
    Paused,
    Finished,
    Stopped,
    Failed,
}
