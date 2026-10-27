//! `GET /ws/runs/:id`. Protocol in `docs/API.md`.
//!
//! Frames and events share one broadcast channel, but events also live in a per-run log with
//! indices. A session that lags (`RecvError::Lagged`) re-reads the log from the last event it
//! sent, so frames can be skipped while events never are.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use mm_core::{EndReason, Event, Metrics, RunState};
use tokio::sync::broadcast::error::RecvError;

use crate::error::ApiError;
use crate::runner::{LiveMsg, RunHandle};
use crate::state::AppState;
use crate::wire::{RunDetail, WsClientMessage, WsServerMessage};

pub fn routes() -> Router<AppState> {
    Router::new().route("/ws/runs/:id", get(upgrade))
}

async fn upgrade(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let run = state.store.get_run(&id)?;
    let maze = state.store.get_maze(&run.maze_id)?;
    Ok(ws.on_upgrade(move |socket| session(state, run, maze, socket)))
}

async fn send(socket: &mut WebSocket, msg: &WsServerMessage) -> bool {
    match serde_json::to_string(msg) {
        Ok(s) => socket.send(Message::Text(s)).await.is_ok(),
        Err(e) => {
            tracing::error!("ws encode failed: {e}");
            false
        }
    }
}

async fn send_events(socket: &mut WebSocket, events: Vec<Event>) -> bool {
    for e in events {
        let m = WsServerMessage::Event {
            t: e.t,
            event: e.kind,
        };
        if !send(socket, &m).await {
            return false;
        }
    }
    true
}

async fn send_end(socket: &mut WebSocket, reason: EndReason, metrics: Metrics) {
    send(socket, &WsServerMessage::End { reason, metrics }).await;
    // Close handshake: give the client a moment to echo the close frame so it does not see a reset.
    if socket.send(Message::Close(None)).await.is_ok() {
        let drain = async { while let Some(Ok(_)) = socket.recv().await {} };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), drain).await;
    }
}

async fn session(
    state: AppState,
    mut run: RunDetail,
    maze: mm_core::MazeGraph,
    mut socket: WebSocket,
) {
    let live = state.runs.get(&run.id);
    // Subscribe before reading anything else so no message can slip between.
    let rx = live.as_ref().map(|h| h.tx.subscribe());
    let (Some(h), Some(rx)) = (live, rx) else {
        // Not live: hello, end, close. Reload so a run that just finished is current.
        if let Ok(fresh) = state.store.get_run(&run.id) {
            run = fresh;
        }
        let reason = run.end_reason.unwrap_or(EndReason::ControllerFault);
        let metrics = run.metrics.clone().unwrap_or_default();
        if send(&mut socket, &WsServerMessage::Hello { run, maze }).await {
            send_end(&mut socket, reason, metrics).await;
        }
        return;
    };
    run.state = h.state();
    if !send(&mut socket, &WsServerMessage::Hello { run, maze }).await {
        return;
    }
    live_session(&h, rx, socket).await;
}

async fn live_session(
    h: &Arc<RunHandle>,
    mut rx: tokio::sync::broadcast::Receiver<LiveMsg>,
    mut socket: WebSocket,
) {
    let past = h.events_range(0, None);
    let mut sent = past.len();
    if !send_events(&mut socket, past).await {
        return;
    }
    if let Some(f) = h.latest_frame() {
        if !send(&mut socket, &WsServerMessage::Frame { frame: f }).await {
            return;
        }
    }
    if let Some((reason, metrics)) = h.ended() {
        let rest = h.events_range(sent, None);
        if send_events(&mut socket, rest).await {
            send_end(&mut socket, reason, metrics).await;
        }
        return;
    }
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(t))) => control(h, &t),
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
            msg = rx.recv() => match msg {
                Ok(LiveMsg::Frame(f)) => {
                    if !send(&mut socket, &WsServerMessage::Frame { frame: (*f).clone() }).await {
                        return;
                    }
                }
                Ok(LiveMsg::Event { index, event }) => {
                    if index >= sent {
                        let missed = h.events_range(sent, Some(index));
                        let mut batch = missed;
                        batch.push((*event).clone());
                        sent = index + 1;
                        if !send_events(&mut socket, batch).await {
                            return;
                        }
                    }
                }
                Ok(LiveMsg::State(s)) => {
                    if s != RunState::Queued && !send(&mut socket, &WsServerMessage::State { state: s }).await {
                        return;
                    }
                }
                Ok(LiveMsg::End) | Err(RecvError::Closed) => {
                    let rest = h.events_range(sent, None);
                    if !send_events(&mut socket, rest).await {
                        return;
                    }
                    let (reason, metrics) = h
                        .ended()
                        .unwrap_or((EndReason::ControllerFault, Metrics::default()));
                    send_end(&mut socket, reason, metrics).await;
                    return;
                }
                Err(RecvError::Lagged(n)) => {
                    let rest = h.events_range(sent, None);
                    sent += rest.len();
                    if !send_events(&mut socket, rest).await
                        || !send(&mut socket, &WsServerMessage::Dropped { count: n }).await
                    {
                        return;
                    }
                }
            },
        }
    }
}

fn control(h: &RunHandle, text: &str) {
    match serde_json::from_str::<WsClientMessage>(text) {
        Ok(WsClientMessage::Stop) => {
            h.request_stop();
        }
        Ok(WsClientMessage::Pause) => {
            h.request_pause();
        }
        Ok(WsClientMessage::Resume) => {
            h.request_resume();
        }
        Err(e) => tracing::debug!("ignoring bad ws message: {e}"),
    }
}
