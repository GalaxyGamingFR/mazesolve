//! MazeMuse HTTP server. `app` builds the whole router so tests can drive it in-process.

pub mod error;
pub mod export;
pub mod routes;
pub mod runner;
pub mod state;
pub mod store;
pub mod wire;

use axum::http::{header, HeaderValue, Method};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub use error::ApiError;
pub use state::AppState;

/// Origins allowed to call the API from a browser.
pub const ALLOWED_ORIGINS: [&str; 2] = ["http://localhost:5173", "http://127.0.0.1:5173"];

fn cors() -> CorsLayer {
    let origins: Vec<HeaderValue> = ALLOWED_ORIGINS
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::ACCEPT])
}

pub fn app(state: AppState) -> Router {
    routes::router()
        .fallback(routes::fallback)
        .with_state(state)
        .layer(cors())
        .layer(TraceLayer::new_for_http())
}
