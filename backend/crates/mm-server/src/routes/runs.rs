use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use mm_core::{CreateRun, Metrics, RngMode, RngSpec, RunState};
use serde::Deserialize;
use tokio::sync::Semaphore;

use super::not_implemented;
use crate::error::{ApiError, ApiJson};
use crate::runner::{launch, Outcome, RunHandle};
use crate::state::AppState;
use crate::wire::{
    BatchRequest, BatchResponse, BatchRun, BatchSummary, RunCreated, RunDetail, RunSummary,
    StateBody,
};

/// Most seeds one batch request may carry.
const MAX_BATCH: usize = 500;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/runs", post(create).get(list))
        .route("/api/runs/batch", post(batch))
        .route("/api/runs/:id", get(get_one))
        .route("/api/runs/:id/pause", post(pause))
        .route("/api/runs/:id/resume", post(resume))
        .route("/api/runs/:id/stop", post(stop))
        .route("/api/runs/:id/replay", get(replay))
        .route("/api/runs/:id/metrics", get(metrics))
        .route("/api/compare", post(not_implemented))
        .route("/api/batch", post(not_implemented))
}

async fn create(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<CreateRun>,
) -> Result<Json<RunCreated>, ApiError> {
    let l = launch(&state, &req).await?;
    Ok(Json(RunCreated {
        id: l.id,
        seed: l.seed,
        state: RunState::Queued,
    }))
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<u32>,
}

async fn list(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<RunSummary>>, ApiError> {
    Ok(Json(
        state
            .store
            .list_runs(q.limit.unwrap_or(50).clamp(1, 1000))?,
    ))
}

async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunDetail>, ApiError> {
    Ok(Json(state.store.get_run(&id)?))
}

async fn metrics(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Metrics>, ApiError> {
    state
        .store
        .get_run(&id)?
        .metrics
        .map(Json)
        .ok_or_else(|| ApiError::Conflict("run has no metrics yet".into()))
}

#[derive(Deserialize)]
struct ReplayQuery {
    format: Option<String>,
}

async fn replay(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ReplayQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let want_msgpack = match q.format.as_deref() {
        Some("msgpack") => true,
        Some("json") => false,
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "format must be json or msgpack, got '{other}'"
            )))
        }
        None => headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.contains("application/msgpack")),
    };
    let blob = state
        .store
        .get_replay_blob(&id)?
        .ok_or_else(|| ApiError::Conflict("run has not produced a replay yet".into()))?;
    if want_msgpack {
        return Ok(([(header::CONTENT_TYPE, "application/msgpack")], blob).into_response());
    }
    let json = tokio::task::spawn_blocking(move || {
        mm_sim::decode_msgpack(&blob).and_then(|r| mm_sim::to_json(&r))
    })
    .await
    .map_err(|e| ApiError::Internal(format!("worker failed: {e}")))?
    .map_err(|e| ApiError::Internal(format!("replay: {e}")))?;
    Ok(([(header::CONTENT_TYPE, "application/json")], json).into_response())
}

/// 404 for unknown ids, 409 when the run is not live any more.
fn live(state: &AppState, id: &str) -> Result<Arc<RunHandle>, ApiError> {
    let run = state.store.get_run(id)?;
    state
        .runs
        .get(id)
        .ok_or_else(|| ApiError::Conflict(format!("run is {:?}, not live", run.state)))
}

async fn pause(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<StateBody>, ApiError> {
    let h = live(&state, &id)?;
    if !h.request_pause() {
        return Err(ApiError::Conflict("run is already paused".into()));
    }
    Ok(Json(StateBody {
        state: RunState::Paused,
    }))
}

async fn resume(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<StateBody>, ApiError> {
    let h = live(&state, &id)?;
    if !h.request_resume() {
        return Err(ApiError::Conflict("run is not paused".into()));
    }
    Ok(Json(StateBody {
        state: RunState::Running,
    }))
}

/// Requests the stop and waits (up to 5 s) for the result to be stored.
async fn stop(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<StateBody>, ApiError> {
    let h = live(&state, &id)?;
    h.request_stop();
    for _ in 0..1000 {
        if state.runs.get(&id).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(Json(StateBody {
        state: state.store.get_run(&id)?.state,
    }))
}

async fn batch(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<BatchRequest>,
) -> Result<Json<BatchResponse>, ApiError> {
    if req.seeds.is_empty() || req.seeds.len() > MAX_BATCH {
        return Err(ApiError::BadRequest(format!(
            "seeds must hold 1 to {MAX_BATCH} entries"
        )));
    }
    let permits = std::thread::available_parallelism().map_or(2, |n| n.get());
    let sem = Arc::new(Semaphore::new(permits));
    let mut pending = Vec::new();
    for seed in req.seeds {
        let mut cfg = req.base.clone();
        cfg.rng = RngSpec {
            mode: RngMode::Fixed,
            seed: Some(seed),
        };
        cfg.speed = 0.0;
        let permit = sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| ApiError::Internal("batch semaphore closed".into()))?;
        let l = launch(&state, &cfg).await?;
        let done = l.done;
        let task = tokio::spawn(async move {
            let o = done.await.ok();
            drop(permit);
            o
        });
        pending.push((l.id, l.seed, task));
    }
    let mut runs = Vec::new();
    for (id, seed, task) in pending {
        let o: Option<Outcome> = task.await.ok().flatten();
        runs.push(BatchRun {
            id,
            seed,
            state: o.map_or(RunState::Failed, |o| o.state),
            score_total: o.and_then(|o| o.score),
        });
    }
    let scores: Vec<i32> = runs.iter().filter_map(|r| r.score_total).collect();
    let summary = BatchSummary {
        count: runs.len() as u32,
        scored: scores.len() as u32,
        mean_score: (!scores.is_empty())
            .then(|| scores.iter().map(|s| f64::from(*s)).sum::<f64>() / scores.len() as f64),
        min_score: scores.iter().copied().min(),
        max_score: scores.iter().copied().max(),
    };
    Ok(Json(BatchResponse { runs, summary }))
}
