# MazeMuse API v0.2.0

Contract between the Rust backend (`backend/`) and the React frontend. The backend lead owns
this file. Frontend changes are proposed in [`API_CHANGES_PROPOSED.md`](API_CHANGES_PROPOSED.md).
Every wire type has a JSON Schema in [`schemas/`](schemas/) (regenerate with
`cargo run -p mm-server --bin mm-schemas`) and mock data lives in [`../fixtures/`](../fixtures/).

## Overview

- Base URL: `http://localhost:8080` (override port with `MM_PORT`). Binds `127.0.0.1` only.
- All bodies are JSON (`Content-Type: application/json`) except where noted (replay msgpack,
  the conversion prompt which is `text/markdown`).
- No authentication. Localhost only. CORS allows exactly `http://localhost:5173` and
  `http://127.0.0.1:5173`.
- **Units on the wire are SI: meters, radians, seconds** (speeds in m/s, rates in rad/s,
  times in seconds since run start, ToF ranges in millimeters as `u16`, as the datasheet does).
  The frontend converts to cm / degrees for display. The only exceptions are fields whose name
  says otherwise (`fov_deg`, `hfov_deg`, `angle_deg`, `*_ms`, `*_mm`, `*_hz`).
- World frame: x east, y north, z up. Yaw 0 points along +x, counter-clockwise positive.
  Tile `(col, row)` covers `[col*t, (col+1)*t] x [row*t, (row+1)*t]`. North is +y.
- Seeds are unsigned integers `< 2^53` so JavaScript can hold them as numbers.
- Ids are opaque strings.

### Error envelope

Every non-2xx response has this body:

```json
{ "error": { "code": "validation_failed", "message": "human readable", "details": null } }
```

| code | HTTP | meaning |
|---|---|---|
| `bad_request` | 400 | malformed JSON, missing field, bad query value |
| `not_found` | 404 | unknown route or id |
| `validation_failed` | 422 | well-formed but violates rules (`details` may hold `ConfigIssue[]`) |
| `conflict` | 409 | wrong state for the action (e.g. pause a finished run) |
| `internal` | 500 | server bug |
| `upstream_unavailable` | 503 | Muse / Meta Model API unreachable or key missing |
| `not_implemented` | 501 | endpoint exists in the contract but is not built yet |

## Endpoints

Status: `live` works now, `planned M2..M6` returns 501 `not_implemented` until that milestone.
Schema links are relative to this file.

| Method | Path | Request | Response | Status |
|---|---|---|---|---|
| GET | `/api/health` | none | health object | live |
| GET | `/api/spec` | none | spec object | live |
| POST | `/api/robots/validate` | [RobotConfig](schemas/RobotConfig.json) | `{valid, issues}` | live |
| POST | `/api/robots` | RobotConfig (with `id` = update, without = create) | RobotConfig with `id` | live |
| GET | `/api/robots` | none | `[{id, name, updated_at}]` | live |
| GET | `/api/robots/:id` | none | RobotConfig | live |
| POST | `/api/mazes/generate` | [GenerateRequest](schemas/GenerateRequest.json) | `{maze, warnings, ascii}` | live |
| POST | `/api/mazes/validate` | [MazeGraph](schemas/MazeGraph.json) | [MazeValidation](schemas/MazeValidation.json) | live |
| GET | `/api/mazes` | none | `[{id, name, seed, w, h, updated_at}]` newest first | live |
| GET | `/api/mazes/:id` | none | MazeGraph | live |
| PUT | `/api/mazes/:id` | MazeGraph | `{maze, validation}` | live |
| GET | `/api/controllers/conversion-prompt` | none | `text/markdown` | planned M4 |
| POST | `/api/controllers/validate` | `{code, lang:"rhai"}` | `{ok, errors, tunables, dry_run_trace?}` | planned M4 |
| POST | `/api/controllers` | `{name, code}` | `{id, version}` | planned M4 |
| GET | `/api/controllers` | none | `[{id, name, version}]` | planned M4 |
| GET | `/api/controllers/:id` | none | `{id, name, version, code, tunables}` | planned M4 |
| POST | `/api/runs` | [CreateRun](schemas/CreateRun.json) | `{run_id, seed, state}` | planned M3 |
| GET | `/api/runs` | none | `[{id, state, seed}]` | planned M3 |
| GET | `/api/runs/:id` | none | `{id, state, config, seed, metrics?}` | planned M3 |
| POST | `/api/runs/:id/pause` | none | `{state}` | planned M3 |
| POST | `/api/runs/:id/resume` | none | `{state}` | planned M3 |
| POST | `/api/runs/:id/stop` | none | `{state}` | planned M3 |
| GET | `/api/runs/:id/replay?format=msgpack\|json` | none | [Replay](schemas/Replay.json) | planned M3 |
| GET | `/api/runs/:id/metrics` | none | [Metrics](schemas/Metrics.json) | planned M3 |
| POST | `/api/compare` | `{run_ids:[...]}` | aligned metrics | planned M5 |
| POST | `/api/batch` | `{base: CreateRun, seeds:[...] \| count}` | `{job_id}` | planned M5 |
| GET | `/api/tuner/params?controller_id=` | none | [TunableParam](schemas/TunableParam.json)`[]` | planned M5 |
| POST | `/api/tuner/test` | none | `{ok, model, latency_ms}` | planned M5 |
| POST | `/api/tuner/suggest` | `{run_id}` | `{job_id}` | planned M5 |
| POST | `/api/tuner/auto` | auto-tune request | `{job_id}` | planned M6 |
| GET | `/api/jobs` | none | `Job[]` | planned M5 |
| GET | `/api/jobs/:id` | none | Job | planned M5 |
| POST | `/api/jobs/:id/cancel` | none | Job | planned M5 |
| WS | `/ws/runs/:id` | see WebSocket | | planned M3 |
| WS | `/ws/jobs/:id` | see WebSocket | | planned M5 |

All 404s for unknown ids use `not_found`; malformed bodies use `bad_request`; rule violations use
`validation_failed`. Unknown routes also return the envelope with `not_found`.
Until an endpoint is built, a plain `GET` to its path returns 501 (WS paths included).

### GET /api/health (live)

```json
{
  "ok": true, "version": "0.1.0", "api_version": "0.2.0",
  "sim_capabilities": {
    "physics": "kinematic",
    "sensors": ["vl53l8cx","as7343","openmv_n6","imx5_tactical","bno055","raw_imu"],
    "rng_modes": ["trng","fixed"],
    "replay_formats": ["msgpack","json"],
    "muse": false
  }
}
```

`muse` is true when `META_MODEL_API_KEY` is set. Example: [`fixtures/health.json`](../fixtures/health.json).

### GET /api/spec (live)

Static catalog the UI builds its forms from. Example: [`fixtures/spec.json`](../fixtures/spec.json).

```json
{
  "api_version": "0.2.0",
  "rules_profiles": [ "RulesProfile (schemas/RulesProfile.json)" ],
  "error_presets": [ "ErrorProfile x3: clean, realistic, chaos (schemas/ErrorProfile.json)" ],
  "reference_robot": "RobotConfig (schemas/RobotConfig.json)",
  "sensor_catalog": [
    { "model": "vl53l8cx", "kind": "tof", "description": "...", "defaults": { "resolution": 8, "fov_deg": 45.0, "rate_hz": 15.0, "max_range_m": 4.0 } }
  ],
  "tunables": []
}
```

`sensor_catalog[].kind` is one of `tof`, `color`, `camera`, `wheel`, `imu`. `defaults` is a free-form
object per model (keys mirror the matching `RobotConfig` mount fields). `tunables` is empty until controllers exist.

### POST /api/robots/validate (live)

Body: `RobotConfig`. Response (always 200 for a well-formed body):

```json
{ "valid": false, "issues": [ { "path": "body.height_m", "severity": "error", "message": "0.3 m exceeds the 0.25 m limit" } ] }
```

`valid` is false when any issue has `severity: "error"`; warnings do not invalidate. Errors: `bad_request`.
Reference robot: [`fixtures/robot_reference.json`](../fixtures/robot_reference.json).

### Robots (live)

- `POST /api/robots` body `RobotConfig`. No `id`: creates (`rob_` + 12 hex chars). With `id`: replaces that robot,
  `not_found` (404) if it does not exist. Any `severity: "error"` issue gives `validation_failed` (422) with
  `details` = the full issue array (same items as `/api/robots/validate`). Returns the stored `RobotConfig` with `id`.
- `GET /api/robots` returns `[{ "id": "rob_reference", "name": "Reference RCJ robot", "updated_at": "2026-10-27T12:00:00Z" }]`
  in creation order. `rob_reference` (the reference robot) always exists; it can be updated like any other.
- `GET /api/robots/:id` returns the `RobotConfig`; `not_found` otherwise.

### Mazes (live)

Mazes are stored server-side (SQLite). Ids are `maz_` + 12 hex chars in `header.id`.

- `POST /api/mazes/generate` body `GenerateRequest` (`seed` null = fresh OS-entropy seed below 2^53). The seed used is
  always echoed in `maze.header.seed`; the same seed and request give an identical maze (apart from `header.id`).
  `name` defaults to `"Maze WxH #seed"`. Response:
  `{ "maze": MazeGraph, "warnings": ConfigIssue[], "ascii": "..." }`. `warnings` is non-empty when fewer features
  than requested could be placed. Errors: `bad_request` (400: unknown `rules` id, grid out of range, impossible features).
  Example: [`fixtures/maze_generate_response.json`](../fixtures/maze_generate_response.json) (8x8, seed 42; the maze alone is
  [`fixtures/maze_generated.json`](../fixtures/maze_generated.json)).
- `POST /api/mazes/validate` body `MazeGraph` (not stored). The server runs tile classification on a copy first, then
  returns `MazeValidation` (`ok`, `issues`, `reachable_tiles`, `enterable_tiles`). Errors: `bad_request` if
  `tiles.length != grid.w * grid.h`.
- `GET /api/mazes` returns `[{ "id", "name", "seed", "w", "h", "updated_at" }]`, newest first.
- `GET /api/mazes/:id` returns the stored `MazeGraph`; `not_found` otherwise.
- `PUT /api/mazes/:id` body `MazeGraph` edited in Maze Studio. The server sets `header.id` from the path, classifies
  tiles, validates, and stores the maze even when validation is not ok. Returns
  `{ "maze": MazeGraph, "validation": MazeValidation }`. Errors: `not_found`, `bad_request` (tile count mismatch).

### Controllers (planned M4)

- `GET /api/controllers/conversion-prompt`: `text/markdown`, the prompt that converts Arduino/C++ robot code to Rhai.
- `POST /api/controllers/validate` `{ "code": "...", "lang": "rhai" }` returns
  `{ "ok": bool, "errors": [{"line": 1, "col": 5, "message": "..."}], "tunables": [TunableParam], "dry_run_trace"?: [...] }`.
- `POST /api/controllers` `{ "name": "wall follower", "code": "..." }` returns `{ "id": "ctl_1", "version": 1 }`
  (saving the same name bumps `version`). Errors: `validation_failed` when the code does not compile.

### Runs (planned M3)

`POST /api/runs` takes `CreateRun`, returns `{ "run_id": "run_1", "seed": 123456, "state": "queued" }`.
`seed` is the resolved seed (drawn from OS entropy when `rng.mode` is `trng`). `speed` 0 means as fast as possible.
`GET /api/runs/:id` returns `{ "id", "state": RunState, "config": CreateRun, "seed", "metrics"? }`; `metrics` appears once
finished. `RunState` is `queued | running | paused | finished | stopped | failed`. `pause`/`resume`/`stop` return
`{ "state" }` or `conflict` if the transition is not valid. `replay?format=` defaults to `json`; `msgpack` is
`application/msgpack`. `404 not_found` for unknown ids; `409 conflict` if the run has not produced a replay yet.

### Compare and batch (planned M5)

`POST /api/compare` `{ "run_ids": ["run_1","run_2"] }` returns `{ "runs": [{ "run_id", "seed", "metrics": Metrics }] , "deltas": {...} }`
with metrics aligned in request order. `POST /api/batch` `{ "base": CreateRun, "seeds": [1,2,3] }` or `{ "base": ..., "count": 20 }`
returns `{ "job_id" }`; the job result is `{ "runs": [{run_id, seed, metrics}], "summary": {mean_score, ...} }`.

### Tuner (planned M5/M6)

- `GET /api/tuner/params?controller_id=` returns `TunableParam[]` (controller `@tunable` values plus allowlisted
  `error_profile.*` robustness settings).
- `POST /api/tuner/test` pings Muse: `{ "ok": true, "model": "muse-spark-1.3", "latency_ms": 412 }`.
  `upstream_unavailable` when no key or the call fails.
- `POST /api/tuner/suggest` `{ "run_id" }` returns `{ "job_id" }`; job `result` is a [MuseReport](schemas/MuseReport.json).
- `POST /api/tuner/auto`:

```json
{ "run_id": "run_1", "max_iterations": 5, "budget_s": 300, "params": ["controller.kp"],
  "allow_code_changes": false, "seed_policy": "same_plus_fresh" }
```

  `seed_policy`: `"same"` re-runs the original seed; `"same_plus_fresh"` also scores a fresh seed to catch overfitting.
  Returns `{ "job_id" }`. Parameter paths outside the allowlist are rejected with `validation_failed`.

### Jobs (planned M5)

`GET /api/jobs/:id` returns `{ "id", "kind": "suggest"|"auto_tune"|"batch", "state": "queued"|"running"|"done"|"failed"|"cancelled",
"progress": 0.0-1.0, "result"?: {...}, "error"?: {code, message} }`. `POST /api/jobs/:id/cancel` returns the job;
`conflict` if already finished.

## WebSocket (planned)

Text frames, one JSON object each, discriminated by `type`.

### `WS /ws/runs/:id`

Server to client:

```json
{ "type": "frame", "t": 1.24, "pose": {"x":0.14,"y":0.14,"z":0,"roll":0,"pitch":0,"yaw":0}, "wheels": [], "imu": {}, "tof": [], "color": [], "cams": [], "id_light": false }
{ "type": "event", "t": 1.3, "kind_type": "tile_enter" }
{ "type": "state", "state": "running" }
{ "type": "dropped", "count": 12 }
{ "type": "end", "metrics": { "...": "Metrics" } }
```

- `frame` is a [Frame](schemas/Frame.json) with `type` added; `event` is an [Event](schemas/Event.json) with `type`
  added. Because `Event` carries its own `type` field (the event kind, e.g. `tile_enter`), the event message nests it:
  `{"type":"event","t":1.3,"event":{"type":"tile_enter","tile":{"col":1,"row":0}}}`. (Decision, see Changelog.)
- Frames stream at about 25 Hz of sim time (scaled by `speed`).
- `dropped` is the backpressure marker: the server skipped `count` frames because the client was slow. Events are never dropped.
- `state` is sent on every `RunState` change; `end` is the last message, then the server closes.

Client to server: `{ "type": "resume", "from_t": 12.5 }` backfills frames and events from the replay starting at
`from_t` (use after reconnecting).

### `WS /ws/jobs/:id`

```json
{ "type": "progress", "progress": 0.4, "iteration": 2, "log_line": "iteration 2: score 85 -> 110" }
{ "type": "done", "result": { } }
{ "type": "error", "code": "upstream_unavailable", "message": "..." }
```

`iteration` and `log_line` are optional. `done` and `error` are terminal.

## Replay format

Replays are served as JSON or MessagePack with the same structure: `{ header, frames, events }`
([Replay](schemas/Replay.json)). Frames are strided at `header.frame_dt_s` (25 Hz); events keep full fidelity.
Same seed gives a byte-identical replay (no wall-clock data except `header.created_at`, set by the server).
Full specification: `docs/REPLAY_FORMAT.md` (coming in M3). Example: [`fixtures/replay_small.json`](../fixtures/replay_small.json).

## Coordination

The frontend must not rely on anything not in this file. To change the contract, add an entry to
[`API_CHANGES_PROPOSED.md`](API_CHANGES_PROPOSED.md); the backend lead accepts, rewrites or rejects it,
then bumps the version here, adds a Changelog line and updates the fixtures in the same commit.

## Changelog

- **0.2.0** (2026-10-27): live: robot create/update/list/get (`rob_reference` seeded), maze generate (response is now `{maze, warnings, ascii}`), validate, list (summaries, not full graphs), get, put (response is `{maze, validation}`). `api_version` is 0.2.0. New fixtures `maze_generated.json`, `maze_generate_response.json`.
- **0.1.0** (2026-10-27): initial draft. Live: health, spec, robot validation. Everything else registered and returning 501.
  Decisions: validation errors use HTTP 422; `event` WebSocket messages nest the `Event` under `event` to avoid clashing
  with `Event.type`; added error code `not_implemented`.
