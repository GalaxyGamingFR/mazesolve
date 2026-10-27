use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use mm_core::{RobotConfig, Severity};
use serde_json::{json, Value};

use super::not_implemented;
use crate::error::ApiJson;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/robots", post(not_implemented).get(not_implemented))
        .route("/api/robots/validate", post(validate))
        .route("/api/robots/:id", get(not_implemented))
}

async fn validate(
    State(state): State<AppState>,
    ApiJson(robot): ApiJson<RobotConfig>,
) -> Json<Value> {
    let issues = robot.validate(&state.rules);
    let valid = issues.iter().all(|i| i.severity != Severity::Error);
    Json(json!({ "valid": valid, "issues": issues }))
}
