use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use mm_core::{ConfigIssue, GenerateRequest, MazeGraph, MazeValidation, RulesProfile};
use mm_maze::MazeError;
use serde_json::{json, Value};

use crate::error::{ApiError, ApiJson};
use crate::state::AppState;
use crate::store::{new_id, trng_seed, MazeSummary};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/mazes/generate", post(generate))
        .route("/api/mazes/validate", post(validate))
        .route("/api/mazes", get(list))
        .route("/api/mazes/:id", get(get_one).put(put_one))
}

/// Body of `POST /api/mazes/generate`.
pub fn generate_response(maze: &MazeGraph, warnings: &[ConfigIssue]) -> Value {
    json!({ "maze": maze, "warnings": warnings, "ascii": mm_maze::ascii(maze) })
}

/// Generates (without storing) a maze with the given seed; sets name but not id.
pub fn generate_maze(
    req: &GenerateRequest,
    seed: u64,
    rules: &RulesProfile,
) -> Result<(MazeGraph, Vec<ConfigIssue>), ApiError> {
    if req.rules != rules.id {
        return Err(ApiError::BadRequest(format!(
            "unknown rules profile '{}'",
            req.rules
        )));
    }
    let (mut maze, warnings) =
        mm_maze::generate_with_report(req, seed, rules).map_err(|e| match e {
            MazeError::BadRequest(m) => ApiError::BadRequest(m),
            MazeError::Internal(m) => ApiError::Internal(m),
        })?;
    maze.header.name = Some(
        req.name
            .clone()
            .unwrap_or_else(|| format!("Maze {}x{} #{}", maze.grid.w, maze.grid.h, seed)),
    );
    Ok((maze, warnings))
}

fn check_shape(maze: &MazeGraph) -> Result<(), ApiError> {
    let n = u64::from(maze.grid.w) * u64::from(maze.grid.h);
    if maze.grid.w == 0 || maze.grid.h == 0 || maze.tiles.len() as u64 != n {
        return Err(ApiError::BadRequest(
            "tiles.len() must equal grid.w * grid.h (both non-zero)".into(),
        ));
    }
    Ok(())
}

async fn generate(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<GenerateRequest>,
) -> Result<Json<Value>, ApiError> {
    let seed = match req.seed {
        Some(s) => s,
        None => trng_seed()?,
    };
    let (mut maze, warnings) = generate_maze(&req, seed, &state.rules)?;
    maze.header.id = Some(new_id("maz")?);
    state.store.insert_maze(&maze)?;
    Ok(Json(generate_response(&maze, &warnings)))
}

async fn validate(
    State(state): State<AppState>,
    ApiJson(mut maze): ApiJson<MazeGraph>,
) -> Result<Json<MazeValidation>, ApiError> {
    check_shape(&maze)?;
    mm_maze::classify(&mut maze);
    Ok(Json(mm_maze::validate(&maze, &state.rules)))
}

async fn list(State(state): State<AppState>) -> Result<Json<Vec<MazeSummary>>, ApiError> {
    Ok(Json(state.store.list_mazes()?))
}

async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<MazeGraph>, ApiError> {
    Ok(Json(state.store.get_maze(&id)?))
}

async fn put_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiJson(mut maze): ApiJson<MazeGraph>,
) -> Result<Json<Value>, ApiError> {
    check_shape(&maze)?;
    let old = state.store.get_maze(&id)?;
    maze.header.id = Some(id.clone());
    if maze.header.name.is_none() {
        maze.header.name = old.header.name;
    }
    mm_maze::classify(&mut maze);
    let validation = mm_maze::validate(&maze, &state.rules);
    state.store.update_maze(&maze)?;
    Ok(Json(json!({ "maze": maze, "validation": validation })))
}
