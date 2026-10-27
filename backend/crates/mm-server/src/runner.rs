//! Run orchestration: resolves a `CreateRun`, spawns the simulation on its own OS thread,
//! paces it against the wall clock and fans frames / events out to WebSocket clients.
//!
//! The server only decides *when* `Sim::step` is called; every number comes from `mm-sim`,
//! so pacing, pausing and listeners never change a replay.

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use mm_controller::{Limits, RhaiController};
use mm_core::{
    CreateRun, EndReason, ErrorProfile, Event, Frame, Metrics, RngMode, RunSettings, RunState,
    SimTiming, SEED_MASK,
};
use mm_sim::{config_hash, ConfigHashes, Sim, SimInput, StepOutcome};
use tokio::sync::{broadcast, oneshot};

use crate::error::ApiError;
use crate::state::AppState;
use crate::store::{new_id, NewRun, RunResult};

/// Smallest gap between two streamed frames of an as-fast-as-possible run (about 30 msgs/s).
const HEADLESS_FRAME_GAP: Duration = Duration::from_millis(33);
/// Broadcast backlog per run before a slow client starts to lag.
const CHANNEL_CAPACITY: usize = 512;

/// What the sim thread publishes to WebSocket sessions.
#[derive(Debug, Clone)]
pub enum LiveMsg {
    Frame(Arc<Frame>),
    /// `index` is the position in the run's event log, so sessions can de-duplicate and
    /// back-fill after lagging.
    Event {
        index: usize,
        event: Arc<Event>,
    },
    State(RunState),
    End,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Shared view of one live run.
pub struct RunHandle {
    pub tx: broadcast::Sender<LiveMsg>,
    stop: AtomicBool,
    pause: AtomicBool,
    state: Mutex<RunState>,
    latest: Mutex<Option<Frame>>,
    events: Mutex<Vec<Event>>,
    ended: Mutex<Option<(EndReason, Metrics)>>,
}

impl RunHandle {
    fn new() -> RunHandle {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        RunHandle {
            tx,
            stop: AtomicBool::new(false),
            pause: AtomicBool::new(false),
            state: Mutex::new(RunState::Queued),
            latest: Mutex::new(None),
            events: Mutex::new(Vec::new()),
            ended: Mutex::new(None),
        }
    }

    pub fn state(&self) -> RunState {
        *lock(&self.state)
    }

    fn set_state(&self, s: RunState) {
        *lock(&self.state) = s;
        let _ = self.tx.send(LiveMsg::State(s));
    }

    pub fn latest_frame(&self) -> Option<Frame> {
        lock(&self.latest).clone()
    }

    /// Events `from..` of the log (all of them so far when `to` is None).
    pub fn events_range(&self, from: usize, to: Option<usize>) -> Vec<Event> {
        let log = lock(&self.events);
        let to = to.unwrap_or(log.len()).min(log.len());
        if from >= to {
            return Vec::new();
        }
        log[from..to].to_vec()
    }

    pub fn ended(&self) -> Option<(EndReason, Metrics)> {
        lock(&self.ended).clone()
    }

    /// False when a stop was already requested.
    pub fn request_stop(&self) -> bool {
        !self.stop.swap(true, Ordering::SeqCst)
    }

    /// False when already paused.
    pub fn request_pause(&self) -> bool {
        !self.pause.swap(true, Ordering::SeqCst)
    }

    /// False when not paused.
    pub fn request_resume(&self) -> bool {
        self.pause.swap(false, Ordering::SeqCst)
    }
}

/// Live runs by id; a handle exists from creation until the result is stored.
#[derive(Default)]
pub struct RunRegistry {
    inner: Mutex<BTreeMap<String, Arc<RunHandle>>>,
}

impl RunRegistry {
    pub fn get(&self, id: &str) -> Option<Arc<RunHandle>> {
        lock(&self.inner).get(id).cloned()
    }

    fn insert(&self, id: &str, h: Arc<RunHandle>) {
        lock(&self.inner).insert(id.to_string(), h);
    }

    fn remove(&self, id: &str) {
        lock(&self.inner).remove(id);
    }
}

/// Final outcome of a run, handed to whoever waits on it (batch).
#[derive(Debug, Clone, Copy)]
pub struct Outcome {
    pub state: RunState,
    pub score: Option<i32>,
}

pub struct Launched {
    pub id: String,
    pub seed: u64,
    pub done: oneshot::Receiver<Outcome>,
}

/// A fully resolved run, ready for its thread.
pub struct Prepared {
    pub id: String,
    pub seed: u64,
    pub controller_version: u32,
    pub speed: f64,
    pub code: String,
    pub input: SimInput,
    pub config: CreateRun,
}

/// Resolves ids to stored configs. 404 for unknown robot, maze, controller or profile.
pub fn prepare(state: &AppState, req: &CreateRun, id: &str) -> Result<Prepared, ApiError> {
    if !req.speed.is_finite() || req.speed < 0.0 {
        return Err(ApiError::BadRequest("speed must be 0 or positive".into()));
    }
    if let Some(d) = req.max_duration_s {
        if !d.is_finite() || d <= 0.0 {
            return Err(ApiError::BadRequest(
                "max_duration_s must be positive".into(),
            ));
        }
    }
    let robot = state.store.get_robot(&req.robot_id)?;
    let maze = state.store.get_maze(&req.maze_id)?;
    let controller = state.store.get_controller(&req.controller_id, None)?;
    let errors = ErrorProfile::preset(&req.error_profile_id)
        .ok_or_else(|| ApiError::NotFound(format!("no error profile {}", req.error_profile_id)))?;
    let seed = match req.rng.mode {
        RngMode::Trng => mm_sim::trng_seed(),
        RngMode::Fixed => {
            req.rng
                .seed
                .ok_or_else(|| ApiError::BadRequest("rng.seed is required for mode fixed".into()))?
                & SEED_MASK
        }
    };
    let hashes = ConfigHashes {
        robot: config_hash(&robot),
        maze: config_hash(&maze),
        controller: config_hash(&controller.code),
        error_profile: config_hash(&errors),
        rules: config_hash(&*state.rules),
    };
    let input = SimInput {
        maze,
        robot,
        rules: (*state.rules).clone(),
        errors,
        settings: RunSettings {
            seed,
            rng_mode: req.rng.mode,
            timing: SimTiming::default(),
            max_duration_s: req.max_duration_s,
            lop_enabled: req.lop_enabled,
        },
        params: req.params.clone(),
        run_id: id.to_string(),
        hashes,
    };
    Ok(Prepared {
        id: id.to_string(),
        seed,
        controller_version: controller.version,
        speed: req.speed,
        code: controller.code,
        input,
        config: req.clone(),
    })
}

/// Stores a queued row and starts the run on a dedicated OS thread (the sim is CPU-bound and
/// must never run on the async runtime). Returns once the sim has been constructed.
pub async fn launch(state: &AppState, req: &CreateRun) -> Result<Launched, ApiError> {
    let id = new_id("run")?;
    let p = prepare(state, req, &id)?;
    let seed = p.seed;
    let created_at = state.store.insert_run(&NewRun {
        id: &id,
        config: &p.config,
        seed,
        controller_version: p.controller_version,
    })?;
    let handle = Arc::new(RunHandle::new());
    state.runs.insert(&id, handle.clone());

    let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();
    let (done_tx, done_rx) = oneshot::channel::<Outcome>();
    let ctx = ThreadCtx {
        state: state.clone(),
        handle,
        created_at,
    };
    let spawned = std::thread::Builder::new()
        .name(format!("sim-{id}"))
        .spawn(move || run_thread(ctx, p, ready_tx, done_tx));
    let cleanup = |state: &AppState| {
        state.runs.remove(&id);
        if let Err(e) = state.store.delete_run(&id) {
            tracing::error!("cleanup of run {id} failed: {e}");
        }
    };
    if let Err(e) = spawned {
        cleanup(state);
        return Err(ApiError::Internal(format!("cannot start run thread: {e}")));
    }
    match ready_rx.await {
        Ok(Ok(())) => Ok(Launched {
            id,
            seed,
            done: done_rx,
        }),
        Ok(Err(msg)) => {
            cleanup(state);
            Err(ApiError::ValidationFailed {
                message: msg,
                details: None,
            })
        }
        Err(_) => {
            cleanup(state);
            Err(ApiError::Internal("run thread died while starting".into()))
        }
    }
}

struct ThreadCtx {
    state: AppState,
    handle: Arc<RunHandle>,
    created_at: String,
}

fn run_thread(
    ctx: ThreadCtx,
    p: Prepared,
    ready: oneshot::Sender<Result<(), String>>,
    done: oneshot::Sender<Outcome>,
) {
    let id = p.id.clone();
    let result = catch_unwind(AssertUnwindSafe(|| execute(&ctx, p, ready)));
    let outcome = match result {
        Ok(o) => o,
        Err(_) => {
            tracing::error!("run {id} panicked");
            Some(finish(
                &ctx,
                &id,
                RunState::Failed,
                EndReason::ControllerFault,
                Metrics::default(),
                None,
            ))
        }
    };
    if let Some(o) = outcome {
        let _ = done.send(o);
    }
}

/// Stores the result, then releases listeners and drops the handle.
fn finish(
    ctx: &ThreadCtx,
    id: &str,
    state: RunState,
    reason: EndReason,
    metrics: Metrics,
    blob: Option<Vec<u8>>,
) -> Outcome {
    let score = metrics.score.total;
    let res = RunResult {
        state,
        end_reason: Some(reason),
        metrics: Some(&metrics),
        replay_blob: blob.as_deref(),
    };
    if let Err(e) = ctx.state.store.finish_run(id, &res) {
        tracing::error!("storing run {id} failed: {e}");
    }
    *lock(&ctx.handle.ended) = Some((reason, metrics));
    *lock(&ctx.handle.state) = state;
    ctx.state.runs.remove(id);
    let _ = ctx.handle.tx.send(LiveMsg::End);
    Outcome {
        state,
        score: Some(score),
    }
}

fn publish_events(h: &RunHandle, sim: &mut Sim) {
    for ev in sim.events_since() {
        let index = {
            let mut log = lock(&h.events);
            log.push(ev.clone());
            log.len() - 1
        };
        let _ = h.tx.send(LiveMsg::Event {
            index,
            event: Arc::new(ev),
        });
    }
}

fn execute(
    ctx: &ThreadCtx,
    p: Prepared,
    ready: oneshot::Sender<Result<(), String>>,
) -> Option<Outcome> {
    let h = &ctx.handle;
    let store = &ctx.state.store;
    let Prepared {
        id,
        speed,
        code,
        input,
        ..
    } = p;
    let mut sim = match Sim::new(input) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(e.to_string()));
            return None;
        }
    };
    let mut ctrl = match RhaiController::new(&code, Limits::default()) {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(format!("controller does not compile: {e}")));
            return None;
        }
    };
    let _ = ready.send(Ok(()));
    h.set_state(RunState::Running);
    if let Err(e) = store.set_run_state(&id, RunState::Running) {
        tracing::error!("run {id}: {e}");
    }

    let frame_dt = sim.header().frame_dt_s.max(1e-3);
    let mut next_frame_t = 0.0;
    let wall0 = Instant::now();
    let mut paused_total = Duration::ZERO;
    let mut pause_start: Option<Instant> = None;
    let mut last_emit = wall0.checked_sub(HEADLESS_FRAME_GAP);
    loop {
        if h.stop.load(Ordering::SeqCst) {
            sim.stop();
            break;
        }
        if h.pause.load(Ordering::SeqCst) {
            if pause_start.is_none() {
                pause_start = Some(Instant::now());
                h.set_state(RunState::Paused);
                let _ = store.set_run_state(&id, RunState::Paused);
            }
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        if let Some(ps) = pause_start.take() {
            paused_total += ps.elapsed();
            h.set_state(RunState::Running);
            let _ = store.set_run_state(&id, RunState::Running);
        }
        if speed > 0.0 {
            let wall = wall0.elapsed().saturating_sub(paused_total).as_secs_f64();
            if sim.time() >= wall * speed {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
        }
        let outcome = sim.step(&mut ctrl);
        publish_events(h, &mut sim);
        if sim.time() >= next_frame_t {
            while next_frame_t <= sim.time() {
                next_frame_t += frame_dt;
            }
            let throttled =
                speed == 0.0 && last_emit.is_some_and(|t| t.elapsed() < HEADLESS_FRAME_GAP);
            if h.tx.receiver_count() > 0 && !throttled {
                last_emit = Some(Instant::now());
                let f = sim.snapshot();
                *lock(&h.latest) = Some(f.clone());
                let _ = h.tx.send(LiveMsg::Frame(Arc::new(f)));
            }
        }
        if let StepOutcome::Ended(_) = outcome {
            break;
        }
    }
    publish_events(h, &mut sim);

    let reason = sim.is_ended().unwrap_or(EndReason::Stopped);
    let mut metrics = sim.metrics();
    let wall = wall0.elapsed().saturating_sub(paused_total).as_secs_f64();
    if speed == 0.0 && wall > 0.0 {
        metrics.realtime_factor = Some(sim.time() / wall);
    }
    let mut replay = sim.into_replay();
    replay.header.created_at = Some(ctx.created_at.clone());
    let (mut state, blob) = match mm_sim::encode_msgpack(&replay) {
        Ok(b) => (RunState::Finished, Some(b)),
        Err(e) => {
            tracing::error!("run {id}: replay encode failed: {e}");
            (RunState::Failed, None)
        }
    };
    match reason {
        EndReason::Stopped if state == RunState::Finished => state = RunState::Stopped,
        EndReason::ControllerFault => state = RunState::Failed,
        _ => {}
    }
    Some(finish(ctx, &id, state, reason, metrics, blob))
}
