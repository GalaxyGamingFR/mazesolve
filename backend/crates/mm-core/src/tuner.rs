//! Tuner types: the parameter allowlist and Muse's structured report.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A parameter the tuner may change. `path` is `controller.<name>` for `@tunable`
/// controller values or `error_profile.<section>.<field>` for robustness settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TunableParam {
    pub path: String,
    pub description: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// Largest change allowed in one iteration, absolute.
    pub max_step: f64,
}

/// The JSON object Muse must return (schema-constrained).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MuseReport {
    pub diagnosis: String,
    pub root_cause: String,
    pub evidence: Vec<String>,
    pub suggestions: Vec<Suggestion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Suggestion {
    pub title: String,
    pub detail: String,
    pub target: SuggestionTarget,
    pub expected_effect: String,
    /// 0..1.
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SuggestionTarget {
    Param { path: String, new_value: f64 },
    Code { diff: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Suggest,
    AutoTune,
    Batch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}
