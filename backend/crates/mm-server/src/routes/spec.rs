use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use mm_core::{ErrorProfile, RobotConfig, RulesProfile, API_VERSION};
use serde_json::{json, Value};

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/spec", get(spec))
}

/// Datasheet-ish defaults for every part the robot builder offers.
pub fn sensor_catalog() -> Value {
    json!([
        {
            "model": "vl53l8cx", "kind": "tof",
            "description": "ST VL53L8CX multizone time-of-flight ranger",
            "defaults": {
                "resolution": 8, "fov_deg": 45.0, "rate_hz": 15.0, "max_range_m": 4.0,
                "resolutions": [4, 8], "max_rate_hz": 60.0, "range_unit": "mm", "i2c_address": "0x52"
            }
        },
        {
            "model": "as7343", "kind": "color",
            "description": "ams OSRAM AS7343 14-channel spectral sensor",
            "defaults": {
                "rate_hz": 20.0, "gain": 16.0, "integration_ms": 20.0, "channels": 14,
                "channel_order": ["F1","F2","F3","F4","F5","F6","F7","F8","FZ","FY","FXL","NIR","Clear","Flicker"]
            }
        },
        {
            "model": "openmv_n6", "kind": "camera",
            "description": "OpenMV N6 smart camera running the victim detector",
            "defaults": { "hfov_deg": 70.0, "vfov_deg": 55.0, "max_range_m": 0.6, "rate_hz": 20.0 }
        },
        {
            "model": "ddsm210", "kind": "wheel",
            "description": "Waveshare DDSM210 hub motor with encoder",
            "defaults": {
                "radius_m": 0.0325, "encoder_ticks_per_rev": 4096, "max_rpm": 210.0,
                "rated_torque_nm": 0.25, "stall_torque_nm": 0.85
            }
        },
        {
            "model": "imx5_tactical", "kind": "imu",
            "description": "Inertial Sense IMX-5, tactical grade, near-zero heading drift",
            "defaults": { "fused_orientation": true, "heading_walk_deg_rthr": 0.05, "rate_hz": 100.0 }
        },
        {
            "model": "bno055", "kind": "imu",
            "description": "Bosch BNO055 fused IMU, moderate heading drift",
            "defaults": { "fused_orientation": true, "heading_walk_deg_rthr": 0.5, "rate_hz": 100.0 }
        },
        {
            "model": "raw_imu", "kind": "imu",
            "description": "MPU-class raw accelerometer and gyro; the controller must fuse",
            "defaults": { "fused_orientation": false, "heading_walk_deg_rthr": 2.0, "rate_hz": 200.0 }
        }
    ])
}

pub fn spec_body(rules: &RulesProfile) -> Value {
    json!({
        "api_version": API_VERSION,
        "rules_profiles": [rules],
        "error_presets": ErrorProfile::presets(),
        "reference_robot": RobotConfig::reference(),
        "sensor_catalog": sensor_catalog(),
        "tunables": [],
    })
}

async fn spec(State(state): State<AppState>) -> Json<Value> {
    Json(spec_body(&state.rules))
}
