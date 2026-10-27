use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use mm_controller::{Limits, RhaiController, ValidationOutcome};
use mm_core::RobotConfig;
use serde::Deserialize;

use crate::error::{ApiError, ApiJson};
use crate::state::AppState;
use crate::wire::{
    ControllerDetail, ControllerSaved, ControllerSummary, ControllerVersion, ConversionPrompt,
    SaveController, ValidateController,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/controllers/conversion-prompt", get(conversion_prompt))
        .route("/api/controllers/validate", post(validate))
        .route("/api/controllers", post(save).get(list))
        .route("/api/controllers/:id", get(get_one))
        .route("/api/controllers/:id/versions", get(versions))
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::Internal(format!("worker failed: {e}")))
}

async fn conversion_prompt() -> Json<ConversionPrompt> {
    Json(ConversionPrompt {
        prompt: mm_controller::CONVERSION_PROMPT.to_string(),
    })
}

async fn validate(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<ValidateController>,
) -> Result<Json<ValidationOutcome>, ApiError> {
    let robot = match &req.robot_id {
        Some(id) => state.store.get_robot(id)?,
        None => RobotConfig::reference(),
    };
    let code = req.code;
    Ok(Json(
        blocking(move || mm_controller::validate(&code, &robot)).await?,
    ))
}

async fn save(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<SaveController>,
) -> Result<Json<ControllerSaved>, ApiError> {
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name must not be empty".into()));
    }
    let code = req.code.clone();
    let compiled = blocking(move || {
        RhaiController::new(&code, Limits::default()).map(|c| c.tunables().to_vec())
    })
    .await?;
    let tunables = compiled.map_err(|e| ApiError::ValidationFailed {
        message: format!("controller does not compile: {e}"),
        details: serde_json::to_value(&e.errors).ok(),
    })?;
    let (id, version) =
        state
            .store
            .save_controller(req.id.as_deref(), &req.name, &req.code, &tunables)?;
    let code = req.code;
    let validation =
        blocking(move || mm_controller::validate(&code, &RobotConfig::reference())).await?;
    Ok(Json(ControllerSaved {
        id,
        version,
        tunables,
        validation,
    }))
}

async fn list(State(state): State<AppState>) -> Result<Json<Vec<ControllerSummary>>, ApiError> {
    Ok(Json(state.store.list_controllers()?))
}

#[derive(Deserialize)]
struct VersionQuery {
    version: Option<u32>,
}

async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<VersionQuery>,
) -> Result<Json<ControllerDetail>, ApiError> {
    Ok(Json(state.store.get_controller(&id, q.version)?))
}

async fn versions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ControllerVersion>>, ApiError> {
    Ok(Json(state.store.controller_versions(&id)?))
}
