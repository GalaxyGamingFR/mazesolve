use axum::routing::{get, post};
use axum::Router;

use super::not_implemented;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/mazes/generate", post(not_implemented))
        .route("/api/mazes/validate", post(not_implemented))
        .route("/api/mazes", get(not_implemented))
        .route("/api/mazes/:id", get(not_implemented).put(not_implemented))
}
