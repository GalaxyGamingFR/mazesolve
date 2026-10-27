//! Error injection knobs for every sensor and actuator, plus the three presets.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorProfile {
    pub id: String,
    pub name: String,
    pub tof: TofErrors,
    pub color: ColorErrors,
    pub camera: CameraErrors,
    pub imu: ImuErrors,
    pub wheel: WheelErrors,
    pub field: FieldErrors,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TofErrors {
    pub sigma_base_mm: f64,
    /// Extra sigma per meter of range.
    pub sigma_per_m_mm: f64,
    /// Probability a zone reports no target.
    pub dropout: f64,
    /// Probability per frame of a close-range ghost/crosstalk reading.
    pub crosstalk: f64,
    /// Probability per frame a whole sensor is blinded by a light flash.
    pub blind: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ColorErrors {
    /// Per-channel noise as a fraction of the reading.
    pub sigma_frac: f64,
    /// Multiplier range on ambient light, applied per run: [1-x, 1+x].
    pub ambient_jitter: f64,
    pub flicker: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CameraErrors {
    pub recall: f64,
    /// Probability per frame of a false positive when nothing is in view.
    pub false_positive: f64,
    /// Probability a decoy is reported as a victim.
    pub decoy_confusion: f64,
    /// Probability a smiley's status is misread.
    pub class_confusion: f64,
    /// Probability per ring that its colour is misread.
    pub ring_confusion: f64,
    pub latency_ms: f64,
    /// Above this yaw rate (rad/s) recall drops to `blur_recall`.
    pub blur_omega: f64,
    pub blur_recall: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImuErrors {
    /// Scale on the preset's datasheet noise (1.0 = datasheet-typical, 0 = perfect).
    pub noise_scale: f64,
    /// Heading random walk, degrees per sqrt(hour), on top of the preset.
    pub heading_walk_deg_rthr: f64,
    /// Probability per second of a magnetic-interference event.
    pub mag_event_rate: f64,
    pub mag_event_deg: f64,
    /// Heading spike (deg) on bumps, stairs and collisions.
    pub vibration_deg: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WheelErrors {
    /// Commanded speeds below this (m/s) produce no motion.
    pub deadband_mps: f64,
    pub max_accel_mps2: f64,
    /// Random per-wheel speed error, fraction.
    pub speed_sigma_frac: f64,
    /// Slip fraction by surface.
    pub slip_normal: f64,
    pub slip_blue: f64,
    pub slip_ramp: f64,
    /// Heading kick (rad) on a wall collision.
    pub collision_yaw_kick: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FieldErrors {
    /// Fraction of the rules' tolerance actually used (1.0 = the full ±10 %).
    pub tolerance_use: f64,
}

impl ErrorProfile {
    pub fn clean() -> Self {
        ErrorProfile {
            id: "clean".into(),
            name: "Clean (no errors)".into(),
            tof: TofErrors {
                sigma_base_mm: 0.0,
                sigma_per_m_mm: 0.0,
                dropout: 0.0,
                crosstalk: 0.0,
                blind: 0.0,
            },
            color: ColorErrors {
                sigma_frac: 0.0,
                ambient_jitter: 0.0,
                flicker: false,
            },
            camera: CameraErrors {
                recall: 1.0,
                false_positive: 0.0,
                decoy_confusion: 0.0,
                class_confusion: 0.0,
                ring_confusion: 0.0,
                latency_ms: 0.0,
                blur_omega: f64::MAX,
                blur_recall: 1.0,
            },
            imu: ImuErrors {
                noise_scale: 0.0,
                heading_walk_deg_rthr: 0.0,
                mag_event_rate: 0.0,
                mag_event_deg: 0.0,
                vibration_deg: 0.0,
            },
            wheel: WheelErrors {
                deadband_mps: 0.0,
                max_accel_mps2: 100.0,
                speed_sigma_frac: 0.0,
                slip_normal: 0.0,
                slip_blue: 0.0,
                slip_ramp: 0.0,
                collision_yaw_kick: 0.0,
            },
            field: FieldErrors { tolerance_use: 0.0 },
        }
    }

    /// Datasheet-typical noise.
    pub fn realistic() -> Self {
        ErrorProfile {
            id: "realistic".into(),
            name: "Realistic (datasheet-typical)".into(),
            tof: TofErrors {
                sigma_base_mm: 3.0,
                sigma_per_m_mm: 5.0,
                dropout: 0.01,
                crosstalk: 0.005,
                blind: 0.001,
            },
            color: ColorErrors {
                sigma_frac: 0.02,
                ambient_jitter: 0.1,
                flicker: false,
            },
            camera: CameraErrors {
                recall: 0.92,
                false_positive: 0.002,
                decoy_confusion: 0.1,
                class_confusion: 0.03,
                ring_confusion: 0.02,
                latency_ms: 60.0,
                blur_omega: 2.0,
                blur_recall: 0.5,
            },
            imu: ImuErrors {
                noise_scale: 1.0,
                heading_walk_deg_rthr: 0.0,
                mag_event_rate: 0.01,
                mag_event_deg: 3.0,
                vibration_deg: 0.5,
            },
            wheel: WheelErrors {
                deadband_mps: 0.005,
                max_accel_mps2: 2.0,
                speed_sigma_frac: 0.02,
                slip_normal: 0.02,
                slip_blue: 0.1,
                slip_ramp: 0.08,
                collision_yaw_kick: 0.03,
            },
            field: FieldErrors { tolerance_use: 1.0 },
        }
    }

    /// Stress test.
    pub fn chaos() -> Self {
        let mut p = Self::realistic();
        p.id = "chaos".into();
        p.name = "Chaos (stress test)".into();
        p.tof = TofErrors {
            sigma_base_mm: 10.0,
            sigma_per_m_mm: 20.0,
            dropout: 0.08,
            crosstalk: 0.03,
            blind: 0.01,
        };
        p.color = ColorErrors {
            sigma_frac: 0.08,
            ambient_jitter: 0.35,
            flicker: true,
        };
        p.camera.recall = 0.75;
        p.camera.false_positive = 0.01;
        p.camera.decoy_confusion = 0.3;
        p.camera.class_confusion = 0.1;
        p.camera.ring_confusion = 0.08;
        p.camera.latency_ms = 150.0;
        p.imu = ImuErrors {
            noise_scale: 3.0,
            heading_walk_deg_rthr: 2.0,
            mag_event_rate: 0.05,
            mag_event_deg: 10.0,
            vibration_deg: 2.0,
        };
        p.wheel.speed_sigma_frac = 0.06;
        p.wheel.slip_normal = 0.06;
        p.wheel.slip_blue = 0.25;
        p.wheel.slip_ramp = 0.2;
        p.wheel.collision_yaw_kick = 0.1;
        p
    }

    pub fn preset(id: &str) -> Option<Self> {
        match id {
            "clean" => Some(Self::clean()),
            "realistic" => Some(Self::realistic()),
            "chaos" => Some(Self::chaos()),
            _ => None,
        }
    }

    pub fn presets() -> Vec<Self> {
        vec![Self::clean(), Self::realistic(), Self::chaos()]
    }
}
