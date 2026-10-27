use axum::routing::{get, post};
use axum::Router;

use super::not_implemented;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/controllers/conversion-prompt", get(not_implemented))
        .route("/api/controllers/validate", post(not_implemented))
        .route(
            "/api/controllers",
            post(not_implemented).get(not_implemented),
        )
        .route("/api/controllers/:id", get(not_implemented))
}
