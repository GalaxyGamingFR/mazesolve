//! SQLite storage. One connection behind a `Mutex`; WAL on disk. Timestamps and ids here
//! are bookkeeping only and never reach the simulation.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use mm_core::{MazeGraph, RobotConfig};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::ApiError;

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
