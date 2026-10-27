use axum::routing::get;
use axum::{Json, Router};
use mm_core::API_VERSION;
use serde_json::{json, Value};

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/health", get(health))
}

pub fn health_body(muse: bool) -> Value {
    json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "api_version": API_VERSION,
        "sim_capabilities": {
            "physics": "kinematic",
            "sensors": ["vl53l8cx", "as7343", "openmv_n6", "imx5_tactical", "bno055", "raw_imu"],
            "rng_modes": ["trng", "fixed"],
            "replay_formats": ["msgpack", "json"],
            "muse": muse,
        }
    })
}

async fn health() -> Json<Value> {
    let muse = std::env::var("META_MODEL_API_KEY").is_ok_and(|v| !v.is_empty());
    Json(health_body(muse))
}
