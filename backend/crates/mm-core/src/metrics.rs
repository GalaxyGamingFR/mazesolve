//! Scoring breakdown and run metrics.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ScoreBreakdown {
    pub victims: i32,
    pub kits: i32,
    pub checkpoints: i32,
    pub ramps: i32,
    pub stairs: i32,
    pub speed_bumps: i32,
    pub blue_tiles: i32,
    pub reliability: i32,
    pub exit_bonus: i32,
    /// Misidentifications, negative.
    pub penalties: i32,
    /// Never below 0 (5.6.14).
    pub total: i32,
}

impl ScoreBreakdown {
    pub fn sum(&mut self) {
        self.total = (self.victims
            + self.kits
            + self.checkpoints
            + self.ramps
            + self.stairs
            + self.speed_bumps
            + self.blue_tiles
            + self.reliability
            + self.exit_bonus
            + self.penalties)
            .max(0);
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Metrics {
    pub score: ScoreBreakdown,
    pub duration_s: f64,
    pub exited: bool,
    pub restarts: u32,
    pub collisions: u32,
    pub tiles_visited: u32,
    pub tiles_reachable: u32,
    /// tiles_visited / tiles_reachable.
    pub coverage: f64,
    pub victims_total: u32,
    pub victims_identified: u32,
    pub misidentifications: u32,
    pub decoy_false_positives: u32,
    pub kits_used: u32,
    pub distance_m: f64,
    /// Sim seconds per wall-clock second (headless speed), if measured.
    pub realtime_factor: Option<f64>,
}
