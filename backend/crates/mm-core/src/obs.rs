//! What a controller sees each step, and what it sends back. No ground truth here:
//! everything is a sensor reading with its error model already applied.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{RingColor, VictimStatus};

/// VL53L8CX zone status, datasheet-shaped (subset).
pub mod tof_status {
    pub const OK: u8 = 5;
    pub const SIGMA_FAIL: u8 = 6;
    pub const WRAP: u8 = 7;
    pub const NO_TARGET: u8 = 255;
}

/// Range sentinel for a zone with no target.
pub const TOF_OUT_OF_RANGE_MM: u16 = 0xFFFF;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Observations {
    /// Sim time, seconds since run start.
    pub t: f64,
    pub tof: Vec<TofReading>,
    pub color: Vec<ColorReading>,
    pub cams: Vec<CamDetection>,
    pub imu: ImuReading,
    /// Cumulative encoder ticks per wheel (FL, FR, RL, RR).
    pub encoders: [i64; 4],
    pub kits_left: u32,
    /// True for one step after a Lack-of-Progress restart.
    pub lop_restarted: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct TofReading {
    pub sensor_id: String,
    /// Row-major zones, `resolution^2` long. Millimeters.
    pub zones: Vec<u16>,
    pub status: Vec<u8>,
    /// True when this step carries a new sample.
    pub fresh: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ColorReading {
    pub sensor_id: String,
    /// AS7343 order: F1..F8, FZ, FY, FXL, NIR, Clear, Flicker. Raw counts.
    pub ch: [f32; 14],
    pub fresh: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CamDetection {
    pub camera_id: String,
    pub kind: DetectionKind,
    /// Reported status for smileys; for cognitive targets see `rings`.
    pub status: Option<VictimStatus>,
    pub rings: Option<[RingColor; 5]>,
    pub confidence: f64,
    pub distance_m: f64,
    /// Bearing in the camera frame, radians (left positive).
    pub bearing: f64,
    /// Sim time the frame was captured (detections arrive after latency).
    pub captured_t: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DetectionKind {
    Smiley,
    Cognitive,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ImuReading {
    /// Fused roll/pitch/yaw, radians (zeros for `raw_imu`).
    pub roll: f64,
    pub pitch: f64,
    pub yaw: f64,
    /// Body rates, rad/s.
    pub gyro: [f64; 3],
    /// Specific force, m/s^2.
    pub accel: [f64; 3],
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Commands {
    /// Target wheel surface speeds, m/s (FL, FR, RL, RR).
    pub wheel_v: [f64; 4],
    pub id_light: bool,
    /// Deploy this many kits this step (0 = none).
    pub deploy_kits: u32,
    /// Report an identified victim: the controller's claim, scored by the sim.
    pub report: Option<VictimReport>,
    /// Ask the sim to treat the run as finished (robot believes it is on start).
    pub end_run: bool,
    /// Free text kept in the replay for debugging.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VictimReport {
    pub status: VictimStatus,
}

/// Static facts a controller gets once at `init`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RobotInfo {
    pub robot: crate::RobotConfig,
    pub tile_m: f64,
    pub controller_dt_s: f64,
    /// Resolved `@tunable` values for this run.
    pub params: std::collections::BTreeMap<String, f64>,
}

/// Implemented by the Rhai host (`mm-controller`) and by built-in Rust controllers.
/// `step` errors end the run with `EndReason::ControllerFault`.
pub trait Controller: Send {
    fn init(&mut self, info: &RobotInfo) -> Result<(), String>;
    fn step(&mut self, obs: &Observations) -> Result<Commands, String>;
}
