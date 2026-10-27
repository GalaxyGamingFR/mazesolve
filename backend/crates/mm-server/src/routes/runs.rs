use axum::routing::{get, post};
use axum::Router;

use super::not_implemented;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/runs", post(not_implemented).get(not_implemented))
        .route("/api/runs/:id", get(not_implemented))
        .route("/api/runs/:id/pause", post(not_implemented))
        .route("/api/runs/:id/resume", post(not_implemented))
        .route("/api/runs/:id/stop", post(not_implemented))
        .route("/api/runs/:id/replay", get(not_implemented))
        .route("/api/runs/:id/metrics", get(not_implemented))
        .route("/api/compare", post(not_implemented))
        .route("/api/batch", post(not_implemented))
        .route("/ws/runs/:id", get(not_implemented))
}
