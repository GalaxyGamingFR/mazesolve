use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use mm_core::{RobotConfig, Severity};
use serde_json::{json, Value};

use crate::error::{ApiError, ApiJson};
use crate::state::AppState;
use crate::store::RobotSummary;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/robots", post(save).get(list))
        .route("/api/robots/validate", post(validate))
        .route("/api/robots/:id", get(get_one))
}

async fn validate(
    State(state): State<AppState>,
    ApiJson(robot): ApiJson<RobotConfig>,
) -> Json<Value> {
    let issues = robot.validate(&state.rules);
    let valid = issues.iter().all(|i| i.severity != Severity::Error);
    Json(json!({ "valid": valid, "issues": issues }))
}

async fn save(
    State(state): State<AppState>,
    ApiJson(robot): ApiJson<RobotConfig>,
) -> Result<Json<RobotConfig>, ApiError> {
    let issues = robot.validate(&state.rules);
    if issues.iter().any(|i| i.severity == Severity::Error) {
        return Err(ApiError::ValidationFailed {
            message: "robot configuration has errors".into(),
            details: Some(json!(issues)),
        });
    }
    Ok(Json(state.store.save_robot(robot)?))
}

async fn list(State(state): State<AppState>) -> Result<Json<Vec<RobotSummary>>, ApiError> {
    Ok(Json(state.store.list_robots()?))
}

async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RobotConfig>, ApiError> {
    Ok(Json(state.store.get_robot(&id)?))
}
