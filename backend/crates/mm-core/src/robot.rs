//! Robot configuration: body, wheels and every sensor mount.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{CoreError, MountPose};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RobotConfig {
    pub id: Option<String>,
    pub name: String,
    pub body: BodySpec,
    /// Order: front-left, front-right, rear-left, rear-right.
    pub wheels: [WheelSpec; 4],
    pub tof: Vec<TofMount>,
    pub color: Vec<ColorMount>,
    pub cameras: Vec<CameraMount>,
    pub imu: ImuPreset,
    pub kits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BodySpec {
    pub length_m: f64,
    pub width_m: f64,
    pub height_m: f64,
    pub mass_kg: f64,
    /// Radius used for wall collision (circle footprint).
    pub collision_radius_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WheelSpec {
    pub model: WheelModel,
    pub radius_m: f64,
    /// Wheel center, robot frame.
    pub x: f64,
    pub y: f64,
    pub encoder_ticks_per_rev: u32,
    pub max_rpm: f64,
    pub rated_torque_nm: f64,
    pub stall_torque_nm: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WheelModel {
    Ddsm210,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TofMount {
    pub id: String,
    pub model: TofModel,
    pub pose: MountPose,
    /// 4 (4x4) or 8 (8x8).
    pub resolution: u8,
    pub fov_deg: f64,
    pub rate_hz: f64,
    pub max_range_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TofModel {
    Vl53l8cx,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ColorMount {
    pub id: String,
    pub model: ColorModel,
    pub pose: MountPose,
    pub rate_hz: f64,
    pub gain: f64,
    pub integration_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ColorModel {
    As7343,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CameraMount {
    pub id: String,
    pub model: CameraModel,
    pub pose: MountPose,
    pub hfov_deg: f64,
    pub vfov_deg: f64,
    pub max_range_m: f64,
    pub rate_hz: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CameraModel {
    OpenmvN6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImuPreset {
    /// Inertial Sense IMX-5, tactical grade, near-zero drift.
    Imx5Tactical,
    /// Bosch BNO055 fused output, moderate heading drift.
    Bno055,
    /// MPU-class raw accel/gyro; the controller must fuse.
    RawImu,
}

/// One problem found by `RobotConfig::validate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigIssue {
    pub path: String,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

impl RobotConfig {
    /// The default robot from the project brief: 8x VL53L8CX, 1x AS7343,
    /// 2x OpenMV N6, 4x DDSM210, IMX-5.
    pub fn reference() -> Self {
        use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};
        let wheel = |x: f64, y: f64| WheelSpec {
            model: WheelModel::Ddsm210,
            radius_m: 0.0325,
            x,
            y,
            encoder_ticks_per_rev: 4096,
            max_rpm: 210.0,
            rated_torque_nm: 0.25,
            stall_torque_nm: 0.85,
        };
        let r = 0.085;
        let tof = (0..8)
            .map(|i| {
                let yaw = i as f64 * FRAC_PI_4;
                TofMount {
                    id: format!("tof{i}"),
                    model: TofModel::Vl53l8cx,
                    pose: MountPose {
                        x: r * yaw.cos(),
                        y: r * yaw.sin(),
                        z: 0.06,
                        yaw,
                        pitch: 0.0,
                    },
                    resolution: 8,
                    fov_deg: 45.0,
                    rate_hz: 15.0,
                    max_range_m: 4.0,
                }
            })
            .collect();
        let cam = |id: &str, yaw: f64| CameraMount {
            id: id.into(),
            model: CameraModel::OpenmvN6,
            pose: MountPose {
                x: 0.0,
                y: 0.09 * yaw.signum(),
                z: 0.07,
                yaw,
                pitch: 0.0,
            },
            hfov_deg: 70.0,
            vfov_deg: 55.0,
            max_range_m: 0.6,
            rate_hz: 20.0,
        };
        RobotConfig {
            id: None,
            name: "Reference RCJ robot".into(),
            body: BodySpec {
                length_m: 0.19,
                width_m: 0.19,
                height_m: 0.12,
                mass_kg: 1.4,
                collision_radius_m: 0.095,
            },
            wheels: [
                wheel(0.06, 0.08),
                wheel(0.06, -0.08),
                wheel(-0.06, 0.08),
                wheel(-0.06, -0.08),
            ],
            tof,
            color: vec![ColorMount {
                id: "color0".into(),
                model: ColorModel::As7343,
                pose: MountPose {
                    x: 0.05,
                    y: 0.0,
                    z: 0.01,
                    yaw: 0.0,
                    pitch: -FRAC_PI_2,
                },
                rate_hz: 20.0,
                gain: 16.0,
                integration_ms: 20.0,
            }],
            cameras: vec![cam("cam_left", FRAC_PI_2), cam("cam_right", -FRAC_PI_2)],
            imu: ImuPreset::Imx5Tactical,
            kits: 12,
        }
    }

    /// Structural checks; rule checks use the rules profile.
    pub fn validate(&self, rules: &crate::RulesProfile) -> Vec<ConfigIssue> {
        let mut out = Vec::new();
        let mut err = |path: &str, msg: String| {
            out.push(ConfigIssue {
                path: path.into(),
                severity: Severity::Error,
                message: msg,
            })
        };
        let b = &self.body;
        if b.height_m > rules.robot.max_height_m {
            err(
                "body.height_m",
                format!(
                    "{} m exceeds the {} m limit",
                    b.height_m, rules.robot.max_height_m
                ),
            );
        }
        if b.collision_radius_m <= 0.0 || b.collision_radius_m * 2.0 >= rules.field.tile_m {
            err(
                "body.collision_radius_m",
                "robot must fit through a single tile".into(),
            );
        }
        for (i, w) in self.wheels.iter().enumerate() {
            if w.radius_m <= 0.0 || w.encoder_ticks_per_rev == 0 {
                err(
                    &format!("wheels[{i}]"),
                    "radius and encoder resolution must be positive".into(),
                );
            }
        }
        let mut ids: Vec<&str> = self.tof.iter().map(|t| t.id.as_str()).collect();
        ids.extend(self.color.iter().map(|c| c.id.as_str()));
        ids.extend(self.cameras.iter().map(|c| c.id.as_str()));
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != ids.len() {
            err("sensors", "sensor ids must be unique".into());
        }
        for (i, t) in self.tof.iter().enumerate() {
            if t.resolution != 4 && t.resolution != 8 {
                err(
                    &format!("tof[{i}].resolution"),
                    "VL53L8CX supports 4x4 or 8x8".into(),
                );
            }
            if t.rate_hz <= 0.0 || t.rate_hz > 60.0 {
                err(
                    &format!("tof[{i}].rate_hz"),
                    "rate must be in (0, 60] Hz".into(),
                );
            }
        }
        if self
            .wheels
            .iter()
            .any(|w| w.rated_torque_nm > w.stall_torque_nm)
        {
            out.push(ConfigIssue {
                path: "wheels".into(),
                severity: Severity::Warning,
                message: "rated torque above stall torque".into(),
            });
        }
        out
    }

    pub fn check(&self, rules: &crate::RulesProfile) -> Result<(), CoreError> {
        match self
            .validate(rules)
            .into_iter()
            .find(|i| i.severity == Severity::Error)
        {
            Some(i) => Err(CoreError::Config(format!("{}: {}", i.path, i.message))),
            None => Ok(()),
        }
    }
}
