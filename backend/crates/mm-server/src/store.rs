//! SQLite storage. One connection behind a `Mutex`; WAL on disk. Timestamps and ids here
//! are bookkeeping only and never reach the simulation.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use mm_core::{CreateRun, EndReason, MazeGraph, Metrics, RobotConfig, RunState, TunableParam};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::ApiError;
use crate::wire::{ControllerDetail, ControllerSummary, ControllerVersion, RunDetail, RunSummary};

pub const REFERENCE_ROBOT_ID: &str = "rob_reference";

const MIGRATION_1: &str = "
CREATE TABLE robots(id TEXT PRIMARY KEY, name TEXT NOT NULL, json TEXT NOT NULL,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE mazes(id TEXT PRIMARY KEY, name TEXT NOT NULL, seed INTEGER NOT NULL,
    w INTEGER NOT NULL, h INTEGER NOT NULL, json TEXT NOT NULL,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE controllers(id TEXT NOT NULL, name TEXT NOT NULL, version INTEGER NOT NULL,
    code TEXT NOT NULL, tunables_json TEXT, created_at TEXT NOT NULL,
    PRIMARY KEY(id, version));
CREATE TABLE runs(id TEXT PRIMARY KEY, state TEXT NOT NULL, config_json TEXT NOT NULL,
    seed INTEGER, metrics_json TEXT, replay_path TEXT, replay_hash TEXT,
    created_at TEXT NOT NULL, finished_at TEXT);
CREATE TABLE jobs(id TEXT PRIMARY KEY, kind TEXT NOT NULL, state TEXT NOT NULL,
    progress REAL NOT NULL DEFAULT 0, request_json TEXT, result_json TEXT, error TEXT,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE tuner_audit(id INTEGER PRIMARY KEY, job_id TEXT NOT NULL, iteration INTEGER,
    kind TEXT NOT NULL, payload_json TEXT, created_at TEXT NOT NULL);
";

const MIGRATION_2: &str = "
ALTER TABLE runs ADD COLUMN maze_id TEXT;
ALTER TABLE runs ADD COLUMN controller_id TEXT;
ALTER TABLE runs ADD COLUMN controller_version INTEGER;
ALTER TABLE runs ADD COLUMN end_reason TEXT;
ALTER TABLE runs ADD COLUMN score_total INTEGER;
ALTER TABLE runs ADD COLUMN replay_blob BLOB;
";

pub const IDLE_RHAI: &str = "// Does nothing; the run ends when time is up.\nfn step(obs) {\n    #{ wheel_v: [0.0, 0.0, 0.0, 0.0] }\n}\n";

/// Built-in controllers seeded by migration 2: (id, name, code).
fn builtin_controllers() -> [(&'static str, &'static str, &'static str); 3] {
    [
        (
            "ctl_wall_follower",
            "Wall follower",
            mm_controller::WALL_FOLLOWER_RHAI,
        ),
        (
            "ctl_victim_check",
            "Wall follower + victims",
            mm_controller::VICTIM_CHECK_RHAI,
        ),
        ("ctl_idle", "Idle", IDLE_RHAI),
    ]
}

fn state_str(s: RunState) -> &'static str {
    match s {
        RunState::Queued => "queued",
        RunState::Running => "running",
        RunState::Paused => "paused",
        RunState::Finished => "finished",
        RunState::Stopped => "stopped",
        RunState::Failed => "failed",
    }
}

fn parse_state(s: &str) -> RunState {
    match s {
        "queued" => RunState::Queued,
        "running" => RunState::Running,
        "paused" => RunState::Paused,
        "finished" => RunState::Finished,
        "stopped" => RunState::Stopped,
        _ => RunState::Failed,
    }
}

fn end_reason_str(r: EndReason) -> &'static str {
    match r {
        EndReason::TimeUp => "time_up",
        EndReason::Exited => "exited",
        EndReason::Stopped => "stopped",
        EndReason::ControllerFault => "controller_fault",
    }
}

fn parse_end_reason(s: &str) -> Option<EndReason> {
    match s {
        "time_up" => Some(EndReason::TimeUp),
        "exited" => Some(EndReason::Exited),
        "stopped" => Some(EndReason::Stopped),
        "controller_fault" => Some(EndReason::ControllerFault),
        _ => None,
    }
}

/// Everything `finish_run` persists.
pub struct RunResult<'a> {
    pub state: RunState,
    pub end_reason: Option<EndReason>,
    pub metrics: Option<&'a Metrics>,
    pub replay_blob: Option<&'a [u8]>,
}

/// New run row, state `queued`.
pub struct NewRun<'a> {
    pub id: &'a str,
    pub config: &'a CreateRun,
    pub seed: u64,
    pub controller_version: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RobotSummary {
    pub id: String,
    pub name: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MazeSummary {
    pub id: String,
    pub name: String,
    pub seed: u64,
    pub w: u32,
    pub h: u32,
    pub updated_at: String,
}

pub struct Store {
    conn: Mutex<Connection>,
}

fn db_err(e: impl std::fmt::Display) -> ApiError {
    tracing::error!("database error: {e}");
    ApiError::Internal("database error".into())
}

/// `prefix_` + 12 lowercase hex chars from the OS entropy source.
pub fn new_id(prefix: &str) -> Result<String, ApiError> {
    let mut b = [0u8; 6];
    getrandom::getrandom(&mut b).map_err(|e| ApiError::Internal(format!("entropy: {e}")))?;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!("{prefix}_{hex}"))
}

/// A fresh seed from the OS entropy source, below 2^53.
pub fn trng_seed() -> Result<u64, ApiError> {
    let mut b = [0u8; 8];
    getrandom::getrandom(&mut b).map_err(|e| ApiError::Internal(format!("entropy: {e}")))?;
    Ok(u64::from_le_bytes(b) & mm_core::SEED_MASK)
}

/// RFC 3339 UTC, second precision.
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_rfc3339(secs)
}

pub fn format_rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Store> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn in_memory() -> anyhow::Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> anyhow::Result<Store> {
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < 1 {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(MIGRATION_1)?;
            let mut r = RobotConfig::reference();
            r.id = Some(REFERENCE_ROBOT_ID.into());
            let now = now_rfc3339();
            tx.execute(
                "INSERT INTO robots(id,name,json,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)",
                params![REFERENCE_ROBOT_ID, r.name, serde_json::to_string(&r)?, now],
            )?;
            tx.pragma_update(None, "user_version", 1)?;
            tx.commit()?;
        }
        if version < 2 {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(MIGRATION_2)?;
            let now = now_rfc3339();
            for (id, name, code) in builtin_controllers() {
                let tunables = mm_controller::parse_tunables(code)
                    .map_err(|_| anyhow::anyhow!("built-in controller {id} does not parse"))?;
                tx.execute(
                    "INSERT INTO controllers(id,name,version,code,tunables_json,created_at)
                     VALUES(?1,?2,1,?3,?4,?5)",
                    params![id, name, code, serde_json::to_string(&tunables)?, now],
                )?;
            }
            tx.pragma_update(None, "user_version", 2)?;
            tx.commit()?;
        }
        // Runs interrupted by a previous shutdown can never finish.
        conn.execute(
            "UPDATE runs SET state='failed', finished_at=?1
             WHERE state IN ('queued','running','paused')",
            [now_rfc3339()],
        )?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, ApiError> {
        self.conn
            .lock()
            .map_err(|_| ApiError::Internal("database lock poisoned".into()))
    }

    // ---- robots ----

    /// Creates (no id) or updates (id present, 404 if unknown). Returns the stored config.
    pub fn save_robot(&self, mut robot: RobotConfig) -> Result<RobotConfig, ApiError> {
        let now = now_rfc3339();
        let conn = self.lock()?;
        match robot.id.clone() {
            Some(id) => {
                let json = serde_json::to_string(&robot).map_err(db_err)?;
                let n = conn
                    .execute(
                        "UPDATE robots SET name=?2, json=?3, updated_at=?4 WHERE id=?1",
                        params![id, robot.name, json, now],
                    )
                    .map_err(db_err)?;
                if n == 0 {
                    return Err(ApiError::NotFound(format!("no robot {id}")));
                }
            }
            None => {
                let id = new_id("rob")?;
                robot.id = Some(id.clone());
                let json = serde_json::to_string(&robot).map_err(db_err)?;
                conn.execute(
                    "INSERT INTO robots(id,name,json,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)",
                    params![id, robot.name, json, now],
                )
                .map_err(db_err)?;
            }
        }
        Ok(robot)
    }

    pub fn get_robot(&self, id: &str) -> Result<RobotConfig, ApiError> {
        let conn = self.lock()?;
        let json: Option<String> = conn
            .query_row("SELECT json FROM robots WHERE id=?1", [id], |r| r.get(0))
            .optional()
            .map_err(db_err)?;
        let json = json.ok_or_else(|| ApiError::NotFound(format!("no robot {id}")))?;
        serde_json::from_str(&json).map_err(db_err)
    }

    pub fn list_robots(&self) -> Result<Vec<RobotSummary>, ApiError> {
        let conn = self.lock()?;
        let mut st = conn
            .prepare("SELECT id,name,updated_at FROM robots ORDER BY rowid")
            .map_err(db_err)?;
        let rows = st
            .query_map([], |r| {
                Ok(RobotSummary {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    updated_at: r.get(2)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<Result<_, _>>().map_err(db_err)
    }

    // ---- mazes ----

    /// Inserts a new maze; `maze.header.id` must be set.
    pub fn insert_maze(&self, maze: &MazeGraph) -> Result<(), ApiError> {
        let id = maze
            .header
            .id
            .clone()
            .ok_or_else(|| ApiError::Internal("maze has no id".into()))?;
        let json = serde_json::to_string(maze).map_err(db_err)?;
        let now = now_rfc3339();
        self.lock()?
            .execute(
                "INSERT INTO mazes(id,name,seed,w,h,json,created_at,updated_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?7)",
                params![
                    id,
                    maze.header.name.clone().unwrap_or_default(),
                    maze.header.seed as i64,
                    maze.grid.w,
                    maze.grid.h,
                    json,
                    now
                ],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Replaces a stored maze (404 if unknown); `maze.header.id` must be set.
    pub fn update_maze(&self, maze: &MazeGraph) -> Result<(), ApiError> {
        let id = maze
            .header
            .id
            .clone()
            .ok_or_else(|| ApiError::Internal("maze has no id".into()))?;
        let json = serde_json::to_string(maze).map_err(db_err)?;
        let n = self
            .lock()?
            .execute(
                "UPDATE mazes SET name=?2, seed=?3, w=?4, h=?5, json=?6, updated_at=?7 WHERE id=?1",
                params![
                    id,
                    maze.header.name.clone().unwrap_or_default(),
                    maze.header.seed as i64,
                    maze.grid.w,
                    maze.grid.h,
                    json,
                    now_rfc3339()
                ],
            )
            .map_err(db_err)?;
        if n == 0 {
            return Err(ApiError::NotFound(format!("no maze {id}")));
        }
        Ok(())
    }

    pub fn get_maze(&self, id: &str) -> Result<MazeGraph, ApiError> {
        let conn = self.lock()?;
        let json: Option<String> = conn
            .query_row("SELECT json FROM mazes WHERE id=?1", [id], |r| r.get(0))
            .optional()
            .map_err(db_err)?;
        let json = json.ok_or_else(|| ApiError::NotFound(format!("no maze {id}")))?;
        serde_json::from_str(&json).map_err(db_err)
    }

    /// Newest first.
    pub fn list_mazes(&self) -> Result<Vec<MazeSummary>, ApiError> {
        let conn = self.lock()?;
        let mut st = conn
            .prepare("SELECT id,name,seed,w,h,updated_at FROM mazes ORDER BY rowid DESC")
            .map_err(db_err)?;
        let rows = st
            .query_map([], |r| {
                Ok(MazeSummary {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    seed: r.get::<_, i64>(2)? as u64,
                    w: r.get(3)?,
                    h: r.get(4)?,
                    updated_at: r.get(5)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<Result<_, _>>().map_err(db_err)
    }

    // ---- controllers (every version is kept) ----

    /// Saves a new version. `id` None creates a controller; an unknown `Some(id)` is a 404.
    /// Returns `(id, version)`.
    pub fn save_controller(
        &self,
        id: Option<&str>,
        name: &str,
        code: &str,
        tunables: &[TunableParam],
    ) -> Result<(String, u32), ApiError> {
        let tunables_json = serde_json::to_string(tunables).map_err(db_err)?;
        let now = now_rfc3339();
        let (id, version) = match id {
            Some(id) => {
                let conn = self.lock()?;
                let max: Option<u32> = conn
                    .query_row(
                        "SELECT MAX(version) FROM controllers WHERE id=?1",
                        [id],
                        |r| r.get(0),
                    )
                    .map_err(db_err)?;
                let max = max.ok_or_else(|| ApiError::NotFound(format!("no controller {id}")))?;
                drop(conn);
                (id.to_string(), max + 1)
            }
            None => (new_id("ctl")?, 1),
        };
        self.lock()?
            .execute(
                "INSERT INTO controllers(id,name,version,code,tunables_json,created_at)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                params![id, name, version, code, tunables_json, now],
            )
            .map_err(db_err)?;
        Ok((id, version))
    }

    /// Latest version when `version` is None.
    pub fn get_controller(
        &self,
        id: &str,
        version: Option<u32>,
    ) -> Result<ControllerDetail, ApiError> {
        let conn = self.lock()?;
        let row = conn
            .query_row(
                "SELECT id,name,version,code,tunables_json,created_at FROM controllers
                 WHERE id=?1 AND version=COALESCE(?2,(SELECT MAX(version) FROM controllers WHERE id=?1))",
                params![id, version],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, u32>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(db_err)?;
        let (id, name, version, code, tj, created_at) = row.ok_or_else(|| {
            ApiError::NotFound(match version {
                Some(v) => format!("no controller {id} version {v}"),
                None => format!("no controller {id}"),
            })
        })?;
        let tunables = match tj {
            Some(s) => serde_json::from_str(&s).map_err(db_err)?,
            None => Vec::new(),
        };
        Ok(ControllerDetail {
            id,
            name,
            version,
            code,
            tunables,
            created_at,
        })
    }

    /// Latest version of each controller, oldest controller first.
    pub fn list_controllers(&self) -> Result<Vec<ControllerSummary>, ApiError> {
        let conn = self.lock()?;
        let mut st = conn
            .prepare(
                "SELECT c.id,c.name,c.version,c.created_at FROM controllers c
                 WHERE c.version=(SELECT MAX(version) FROM controllers WHERE id=c.id)
                 ORDER BY (SELECT MIN(rowid) FROM controllers WHERE id=c.id)",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map([], |r| {
                Ok(ControllerSummary {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    version: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<Result<_, _>>().map_err(db_err)
    }

    /// All versions of one controller, newest first.
    pub fn controller_versions(&self, id: &str) -> Result<Vec<ControllerVersion>, ApiError> {
        let conn = self.lock()?;
        let mut st = conn
            .prepare(
                "SELECT version,name,created_at FROM controllers WHERE id=?1 ORDER BY version DESC",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map([id], |r| {
                Ok(ControllerVersion {
                    version: r.get(0)?,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })
            .map_err(db_err)?;
        let v: Vec<ControllerVersion> = rows.collect::<Result<_, _>>().map_err(db_err)?;
        if v.is_empty() {
            return Err(ApiError::NotFound(format!("no controller {id}")));
        }
        Ok(v)
    }

    // ---- runs ----

    pub fn insert_run(&self, run: &NewRun<'_>) -> Result<String, ApiError> {
        let created_at = now_rfc3339();
        let config_json = serde_json::to_string(run.config).map_err(db_err)?;
        self.lock()?
            .execute(
                "INSERT INTO runs(id,state,config_json,seed,maze_id,controller_id,controller_version,created_at)
                 VALUES(?1,'queued',?2,?3,?4,?5,?6,?7)",
                params![
                    run.id,
                    config_json,
                    run.seed as i64,
                    run.config.maze_id,
                    run.config.controller_id,
                    run.controller_version,
                    created_at
                ],
            )
            .map_err(db_err)?;
        Ok(created_at)
    }

    pub fn set_run_state(&self, id: &str, state: RunState) -> Result<(), ApiError> {
        self.lock()?
            .execute(
                "UPDATE runs SET state=?2 WHERE id=?1",
                params![id, state_str(state)],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn finish_run(&self, id: &str, res: &RunResult<'_>) -> Result<(), ApiError> {
        let metrics_json = match res.metrics {
            Some(m) => Some(serde_json::to_string(m).map_err(db_err)?),
            None => None,
        };
        self.lock()?
            .execute(
                "UPDATE runs SET state=?2, end_reason=?3, metrics_json=?4, score_total=?5,
                 replay_blob=?6, finished_at=?7 WHERE id=?1",
                params![
                    id,
                    state_str(res.state),
                    res.end_reason.map(end_reason_str),
                    metrics_json,
                    res.metrics.map(|m| m.score.total),
                    res.replay_blob,
                    now_rfc3339()
                ],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn delete_run(&self, id: &str) -> Result<(), ApiError> {
        self.lock()?
            .execute("DELETE FROM runs WHERE id=?1", [id])
            .map_err(db_err)?;
        Ok(())
    }

    pub fn get_run(&self, id: &str) -> Result<RunDetail, ApiError> {
        let conn = self.lock()?;
        let row = conn
            .query_row(
                "SELECT id,state,config_json,seed,maze_id,controller_id,controller_version,
                        end_reason,metrics_json,created_at,finished_at FROM runs WHERE id=?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<u32>>(6)?,
                        r.get::<_, Option<String>>(7)?,
                        r.get::<_, Option<String>>(8)?,
                        r.get::<_, String>(9)?,
                        r.get::<_, Option<String>>(10)?,
                    ))
                },
            )
            .optional()
            .map_err(db_err)?;
        let (id, state, cfg, seed, maze_id, ctl_id, ctl_ver, reason, metrics, created_at, fin) =
            row.ok_or_else(|| ApiError::NotFound(format!("no run {id}")))?;
        Ok(RunDetail {
            id,
            state: parse_state(&state),
            config: serde_json::from_str(&cfg).map_err(db_err)?,
            seed: seed as u64,
            maze_id: maze_id.unwrap_or_default(),
            controller_id: ctl_id.unwrap_or_default(),
            controller_version: ctl_ver.unwrap_or(1),
            end_reason: reason.as_deref().and_then(parse_end_reason),
            metrics: match metrics {
                Some(s) => Some(serde_json::from_str(&s).map_err(db_err)?),
                None => None,
            },
            created_at,
            finished_at: fin,
        })
    }

    /// Newest first.
    pub fn list_runs(&self, limit: u32) -> Result<Vec<RunSummary>, ApiError> {
        let conn = self.lock()?;
        let mut st = conn
            .prepare(
                "SELECT id,maze_id,controller_id,seed,state,score_total,created_at FROM runs
                 ORDER BY rowid DESC LIMIT ?1",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map([limit], |r| {
                Ok(RunSummary {
                    id: r.get(0)?,
                    maze_id: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    controller_id: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    seed: r.get::<_, i64>(3)? as u64,
                    state: parse_state(&r.get::<_, String>(4)?),
                    score_total: r.get(5)?,
                    created_at: r.get(6)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<Result<_, _>>().map_err(db_err)
    }

    /// The msgpack replay blob; `None` while the run has not finished.
    pub fn get_replay_blob(&self, id: &str) -> Result<Option<Vec<u8>>, ApiError> {
        let conn = self.lock()?;
        let row: Option<Option<Vec<u8>>> = conn
            .query_row("SELECT replay_blob FROM runs WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(db_err)?;
        row.ok_or_else(|| ApiError::NotFound(format!("no run {id}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_formats() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(951_782_400 + 86_399), "2000-02-29T23:59:59Z");
    }

    #[test]
    fn ids_and_seed() {
        let id = new_id("rob").unwrap();
        assert_eq!(id.len(), 16);
        assert!(trng_seed().unwrap() < (1u64 << 53));
    }
}
