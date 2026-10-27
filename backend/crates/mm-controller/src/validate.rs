//! Compile a script and dry-run it against stub observations.

use mm_core::{
    tof_status, ColorReading, Commands, ImuReading, Observations, RobotConfig, RobotInfo,
    TofReading, TunableParam,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::controller::{Limits, RhaiController, ScriptError};
use crate::tunables::parse_tunables;

const DRY_RUN_STEPS: usize = 50;
const DT_S: f64 = 0.01;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TraceStep {
    pub t: f64,
    pub commands: Commands,
    pub log: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ValidationOutcome {
    pub ok: bool,
    pub errors: Vec<ScriptError>,
    pub tunables: Vec<TunableParam>,
    pub dry_run_trace: Vec<TraceStep>,
}

/// Observations shaped from the robot's sensors: zones at 500 mm with OK status, flat
/// colour channels, a level IMU and zero encoders.
pub fn stub_observations(robot: &RobotConfig, t: f64) -> Observations {
    Observations {
        t,
        tof: robot
            .tof
            .iter()
            .map(|m| {
                let n = usize::from(m.resolution) * usize::from(m.resolution);
                TofReading {
                    sensor_id: m.id.clone(),
                    zones: vec![500; n],
                    status: vec![tof_status::OK; n],
                    fresh: true,
                }
            })
            .collect(),
        color: robot
            .color
            .iter()
            .map(|m| ColorReading {
                sensor_id: m.id.clone(),
                ch: [1000.0; 14],
                fresh: true,
            })
            .collect(),
        cams: Vec::new(),
        imu: ImuReading::default(),
        encoders: [0; 4],
        kits_left: robot.kits,
        lop_restarted: false,
    }
}

/// Compile `code`, then run 50 ticks against stub observations.
pub fn validate(code: &str, robot: &RobotConfig) -> ValidationOutcome {
    let mut outcome = ValidationOutcome {
        ok: false,
        errors: Vec::new(),
        tunables: parse_tunables(code).unwrap_or_default(),
        dry_run_trace: Vec::new(),
    };
    let mut ctl = match RhaiController::new(code, Limits::default()) {
        Ok(c) => c,
        Err(e) => {
            outcome.errors = e.errors;
            return outcome;
        }
    };
    let info = RobotInfo {
        robot: robot.clone(),
        tile_m: mm_core::rules_2027_draft().field.tile_m,
        controller_dt_s: DT_S,
        params: Default::default(),
    };
    if let Err(e) = ctl.run_init(&info) {
        outcome.errors.push(e);
        return outcome;
    }
    for i in 0..DRY_RUN_STEPS {
        let t = i as f64 * DT_S;
        match ctl.run_step(&stub_observations(robot, t)) {
            Ok(commands) => outcome.dry_run_trace.push(TraceStep {
                t,
                commands,
                log: ctl.take_log(),
            }),
            Err(mut e) => {
                e.message = format!("dry-run step {i}: {}", e.message);
                outcome.errors.push(e);
                return outcome;
            }
        }
    }
    outcome.ok = true;
    outcome
}
