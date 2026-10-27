//! One module per resource. Endpoints not built yet answer 501 `not_implemented`
//! so the frontend sees the full surface from day one.

pub mod controllers;
pub mod health;
pub mod jobs;
pub mod mazes;
pub mod robots;
pub mod runs;
pub mod spec;
pub mod tuner;

use axum::Router;

use crate::error::ApiError;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(health::routes())
        .merge(spec::routes())
        .merge(robots::routes())
        .merge(mazes::routes())
        .merge(controllers::routes())
        .merge(runs::routes())
        .merge(tuner::routes())
        .merge(jobs::routes())
}

/// Placeholder for endpoints scheduled for a later milestone.
pub async fn not_implemented() -> ApiError {
    ApiError::NotImplemented("this endpoint is planned for a later milestone".into())
}

pub async fn fallback() -> ApiError {
    ApiError::NotFound("no such route".into())
}
