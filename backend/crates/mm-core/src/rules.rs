//! Rules profile: every field and scoring number from the RCJ Rescue Maze rules.
//! Loaded from `rules_profiles/*.json`; logic must never hardcode these values.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::CoreError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RulesProfile {
    /// e.g. `rcj_maze_2027_draft`.
    pub id: String,
    pub name: String,
    pub source_url: String,
    pub field: FieldRules,
    pub scoring: ScoringRules,
    pub play: PlayRules,
    pub robot: RobotRules,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FieldRules {
    pub tile_m: f64,
    pub wall_height_min_m: f64,
    /// Allowed deviation on all measurements, as a fraction (0.10 = ±10 %).
    pub tolerance_frac: f64,
    pub tile_lip_max_m: f64,
    pub ramp_max_deg: f64,
    pub stairs_step_max_m: f64,
    pub stairs_tread_min_m: f64,
    pub stairs_incline_max_deg: f64,
    pub stairs_incline_max_deg_dz: f64,
    pub speed_bump_max_m: f64,
    pub speed_bump_max_m_dz: f64,
    pub obstacle_min_height_m: f64,
    pub obstacle_wall_clearance_m: f64,
    pub victim_diameter_m: f64,
    pub victim_height_m: f64,
    pub cognitive_ring_diameters_m: Vec<f64>,
    /// Ring colour -> value, summed to get the cognitive target's status.
    pub cognitive_ring_values: Vec<RingValue>,
    pub blue_stop_s: f64,
    pub bridge_clearance_min_m: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RingValue {
    pub color: crate::RingColor,
    pub value: i32,
}

/// §5.6 of the 2027 draft. Variable names follow the rules: SVI = successful victim
/// identifications, SRD = successful rescue-kit deployments, SBV = blue tiles visited
/// once, SSN = stairs negotiated, SRN = ramps negotiated, LoP = lack-of-progress count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScoringRules {
    /// 5.6.2: identification points by tile class and victim kind.
    pub victim_points: VictimPoints,
    /// 5.6.3-5: points by number of kits deployed to one victim (index 0 = 1 kit).
    pub kit_points_by_count: Vec<i32>,
    /// Kits that count per victim status.
    pub kits_max: StatusCounts,
    /// 5.6.6: first visit of a blue tile, minus `blue_revisit_penalty` per revisit, floor 0.
    pub blue_tile: i32,
    pub blue_revisit_penalty: i32,
    /// 5.6.7 reliability bonus: SVI*a + SRD*b + SBV*c - LoP*d, floor 0.
    pub reliability_per_svi: i32,
    pub reliability_per_srd: i32,
    pub reliability_per_sbv: i32,
    pub reliability_per_lop: i32,
    pub speed_bump_tile: i32,
    pub ramp: i32,
    pub stairs: i32,
    pub checkpoint: i32,
    /// 5.6.12 exit bonus: SVI*a + SBV*b + SSN*c + SRN*d.
    pub exit_per_svi: i32,
    pub exit_per_sbv: i32,
    pub exit_per_ssn: i32,
    pub exit_per_srn: i32,
    /// 5.6.14, negative.
    pub misidentification: i32,
    /// 5.6.1/5.6.14: identification must happen within this distance of the victim.
    pub identify_within_m: f64,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VictimPoints {
    pub linear_smiley: i32,
    pub linear_cognitive: i32,
    pub floating_smiley: i32,
    pub floating_cognitive: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatusCounts {
    pub harmed: u32,
    pub stable: u32,
    pub unharmed: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlayRules {
    pub run_duration_s: f64,
    /// Sim-only: the rules leave LoP to the captain; we auto-call it after this long
    /// without `lop_min_displacement_m` of net movement.
    pub lop_no_progress_s: f64,
    pub lop_min_displacement_m: f64,
    /// 5.6.1: blink the Identification Light this long while stationary.
    pub identification_light_s: f64,
    pub identification_blink_on_s: f64,
    pub identification_blink_off_s: f64,
    /// 5.6.12: light on, robot still, on the start tile, this long.
    pub exit_light_s: f64,
    /// Kits must land within this distance of the victim.
    pub kit_max_distance_m: f64,
    pub kit_min_dimension_m: f64,
    /// Speed below which the robot counts as stationary (sim-only).
    pub stationary_mps: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RobotRules {
    pub max_height_m: f64,
}

impl RulesProfile {
    pub fn from_json(s: &str) -> Result<Self, CoreError> {
        let p: RulesProfile = serde_json::from_str(s)?;
        p.check()?;
        Ok(p)
    }

    /// Sanity checks so a broken profile fails at load, not mid-run.
    pub fn check(&self) -> Result<(), CoreError> {
        let f = &self.field;
        if f.tile_m <= 0.0 || f.wall_height_min_m <= 0.0 {
            return Err(CoreError::Rules(
                "tile and wall sizes must be positive".into(),
            ));
        }
        if !(0.0..0.5).contains(&f.tolerance_frac) {
            return Err(CoreError::Rules(
                "tolerance_frac must be in [0, 0.5)".into(),
            ));
        }
        if f.cognitive_ring_diameters_m.len() != 5 {
            return Err(CoreError::Rules("cognitive targets have 5 rings".into()));
        }
        if self.play.run_duration_s <= 0.0 {
            return Err(CoreError::Rules("run_duration_s must be positive".into()));
        }
        Ok(())
    }

    pub fn ring_value(&self, c: crate::RingColor) -> i32 {
        self.field
            .cognitive_ring_values
            .iter()
            .find(|r| r.color == c)
            .map_or(0, |r| r.value)
    }
}
