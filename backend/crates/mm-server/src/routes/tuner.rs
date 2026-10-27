use axum::routing::{get, post};
use axum::Router;

use super::not_implemented;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/tuner/params", get(not_implemented))
        .route("/api/tuner/test", post(not_implemented))
        .route("/api/tuner/suggest", post(not_implemented))
        .route("/api/tuner/auto", post(not_implemented))
}
