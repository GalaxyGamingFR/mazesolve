//! Wire types for the controller, run and WebSocket endpoints (JSON Schema is generated
//! from these, see `export.rs`).

use mm_controller::ValidationOutcome;
use mm_core::{CreateRun, EndReason, EventKind, MazeGraph, Metrics, RunState, TunableParam};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---- controllers ----

/// `POST /api/controllers/validate` body.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ValidateController {
    pub code: String,
    /// Only `"rhai"` exists; accepted and ignored.
    #[serde(default)]
    pub lang: Option<String>,
    /// Robot whose sensors shape the dry run; defaults to the reference robot.
    #[serde(default)]
    pub robot_id: Option<String>,
}

/// `POST /api/controllers` body.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SaveController {
    /// Present: save a new version of this controller. Absent: create a controller.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ControllerSaved {
    pub id: String,
    pub version: u32,
    pub tunables: Vec<TunableParam>,
    pub validation: ValidationOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ControllerSummary {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ControllerVersion {
    pub version: u32,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ControllerDetail {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub code: String,
    pub tunables: Vec<TunableParam>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConversionPrompt {
    pub prompt: String,
}

// ---- runs ----

/// `POST /api/runs` response.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct RunCreated {
    pub id: String,
    pub seed: u64,
    pub state: RunState,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct RunSummary {
    pub id: String,
    pub maze_id: String,
    pub controller_id: String,
    pub seed: u64,
    pub state: RunState,
    /// Final score; null until the run has ended.
    pub score_total: Option<i32>,
    pub created_at: String,
}

/// `GET /api/runs/:id` response, also the `run` field of the WS `hello` message.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct RunDetail {
    pub id: String,
    pub state: RunState,
    pub config: CreateRun,
    /// The resolved seed (never null, also for `trng` runs).
    pub seed: u64,
    pub maze_id: String,
    pub controller_id: String,
    pub controller_version: u32,
    pub end_reason: Option<EndReason>,
    /// Present once the run has ended.
    pub metrics: Option<Metrics>,
    pub created_at: String,
    pub finished_at: Option<String>,
}

/// `POST /api/runs/:id/stop|pause|resume` response.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct StateBody {
    pub state: RunState,
}

/// `POST /api/runs/batch` body.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchRequest {
    /// Template; `rng` and `speed` are overridden per run (fixed seed, speed 0).
    pub base: CreateRun,
    pub seeds: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct BatchRun {
    pub id: String,
    pub seed: u64,
    pub state: RunState,
    pub score_total: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct BatchSummary {
    pub count: u32,
    /// Runs that ended with a score (finished or stopped).
    pub scored: u32,
    pub mean_score: Option<f64>,
    pub min_score: Option<i32>,
    pub max_score: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct BatchResponse {
    pub runs: Vec<BatchRun>,
    pub summary: BatchSummary,
}

// ---- WebSocket ----

/// Server to client messages on `/ws/runs/:id`, discriminated by `type`.
// One short-lived value per message; boxing would not change the wire format.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsServerMessage {
    /// First message. `maze` is the stored maze the run was created from.
    Hello {
        run: RunDetail,
        maze: MazeGraph,
    },
    Frame {
        frame: mm_core::Frame,
    },
    /// `event` is the event kind (its own `type` field names the kind); `t` is sim time.
    Event {
        t: f64,
        event: EventKind,
    },
    /// Sent when a live run changes state (pause/resume).
    State {
        state: RunState,
    },
    /// The client was too slow and `count` frames were skipped. Events are never skipped.
    Dropped {
        count: u64,
    },
    /// Last message; the server closes the socket after it.
    End {
        reason: EndReason,
        metrics: Metrics,
    },
}

/// Client to server messages.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsClientMessage {
    Stop,
    Pause,
    Resume,
}
