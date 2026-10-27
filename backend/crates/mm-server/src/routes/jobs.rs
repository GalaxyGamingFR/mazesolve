use axum::routing::{get, post};
use axum::Router;

use super::not_implemented;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/jobs", get(not_implemented))
        .route("/api/jobs/:id", get(not_implemented))
        .route("/api/jobs/:id/cancel", post(not_implemented))
        .route("/ws/jobs/:id", get(not_implemented))
}
