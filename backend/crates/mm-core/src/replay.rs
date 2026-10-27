//! Replay: header + strided frames + full-fidelity events. See `docs/REPLAY_FORMAT.md`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    CamDetection, ColorReading, ImuReading, Pose3, RngMode, ScoreBreakdown, TileXY, TofReading,
    VictimStatus,
};

pub const REPLAY_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Replay {
    pub header: ReplayHeader,
    pub frames: Vec<Frame>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReplayHeader {
    pub format_version: u32,
    pub run_id: String,
    pub seed: u64,
    pub rng_mode: RngMode,
    pub robot_hash: String,
    pub maze_hash: String,
    pub controller_hash: String,
    pub error_profile_hash: String,
    pub rules_hash: String,
    /// Tolerance actually applied to this perturbed maze instance.
    pub applied_tolerance_frac: f64,
    pub physics_dt_s: f64,
    pub controller_dt_s: f64,
    pub frame_dt_s: f64,
    /// Set by the server, never read by the sim (keeps replays byte-identical).
    pub created_at: Option<String>,
    pub backend_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Frame {
    pub t: f64,
    /// Ground-truth pose.
    pub pose: Pose3,
    pub wheels: [WheelState; 4],
    pub imu: ImuReading,
    /// Only sensors that produced a fresh sample since the previous frame.
    pub tof: Vec<TofReading>,
    pub color: Vec<ColorReading>,
    pub cams: Vec<CamDetection>,
    pub id_light: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct WheelState {
    pub cmd: f64,
    pub actual: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    pub t: f64,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    RunStart,
    TileEnter {
        tile: TileXY,
    },
    TileExit {
        tile: TileXY,
    },
    Collision {
        x: f64,
        y: f64,
    },
    Checkpoint {
        tile: TileXY,
    },
    BlueStop {
        tile: TileXY,
        ok: bool,
    },
    VictimIdentified {
        victim_id: u32,
        reported: VictimStatus,
        correct: bool,
        points: i32,
    },
    VictimMissed {
        victim_id: u32,
    },
    Misidentification {
        reported: VictimStatus,
    },
    DecoyFalsePositive {
        victim_id: u32,
    },
    KitDeployed {
        victim_id: Option<u32>,
        count: u32,
        points: i32,
    },
    RampTraversed {
        tile: TileXY,
        up: bool,
        points: i32,
    },
    StairsTraversed {
        tile: TileXY,
        points: i32,
    },
    SpeedBumpTraversed {
        tile: TileXY,
        points: i32,
    },
    EnteredBlack {
        tile: TileXY,
    },
    LopRestart {
        to: TileXY,
        restarts: u32,
    },
    ControllerError {
        message: String,
    },
    Note {
        text: String,
    },
    RunEnd {
        reason: EndReason,
        score: ScoreBreakdown,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    TimeUp,
    Exited,
    Stopped,
    ControllerFault,
}
