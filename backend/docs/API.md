# MazeMuse API v0.3.0

Contract between the Rust backend (`backend/`) and the React frontend. The backend lead owns
this file. Frontend changes are proposed in [`API_CHANGES_PROPOSED.md`](API_CHANGES_PROPOSED.md).
Every wire type has a JSON Schema in [`schemas/`](schemas/) (regenerate with
`cargo run -p mm-server --bin mm-schemas`) and mock data lives in [`../fixtures/`](../fixtures/).

## Overview

- Base URL: `http://localhost:8080` (override port with `MM_PORT`). Binds `127.0.0.1` only.
- All bodies are JSON (`Content-Type: application/json`) except where noted (replay msgpack,
  the conversion prompt which is JSON `{prompt}`).
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
| GET | `/api/controllers/conversion-prompt` | none | `{prompt}` | live |
| POST | `/api/controllers/validate` | [ValidateController](schemas/ValidateController.json) `{code, lang?, robot_id?}` | [ValidationOutcome](schemas/ValidationOutcome.json) | live |
| POST | `/api/controllers` | [SaveController](schemas/SaveController.json) `{id?, name, code}` | [ControllerSaved](schemas/ControllerSaved.json) | live |
| GET | `/api/controllers` | none | [ControllerSummary](schemas/ControllerSummary.json)`[]` (latest versions) | live |
| GET | `/api/controllers/:id?version=` | none | [ControllerDetail](schemas/ControllerDetail.json) | live |
| GET | `/api/controllers/:id/versions` | none | [ControllerVersion](schemas/ControllerVersion.json)`[]` newest first | live |
| POST | `/api/runs` | [CreateRun](schemas/CreateRun.json) | [RunCreated](schemas/RunCreated.json) `{id, seed, state}` | live |
| GET | `/api/runs?limit=` | none | [RunSummary](schemas/RunSummary.json)`[]` newest first (default 50, max 1000) | live |
| GET | `/api/runs/:id` | none | [RunDetail](schemas/RunDetail.json) | live |
| POST | `/api/runs/:id/pause` | none | `{state}` | live |
| POST | `/api/runs/:id/resume` | none | `{state}` | live |
| POST | `/api/runs/:id/stop` | none | `{state}` | live |
| GET | `/api/runs/:id/replay?format=msgpack\|json` | none | [Replay](schemas/Replay.json) | live |
| GET | `/api/runs/:id/metrics` | none | [Metrics](schemas/Metrics.json) | live |
| POST | `/api/runs/batch` | [BatchRequest](schemas/BatchRequest.json) `{base: CreateRun, seeds: [u64]}` | [BatchResponse](schemas/BatchResponse.json) | live |
| POST | `/api/compare` | `{run_ids:[...]}` | aligned metrics | planned M5 |
| POST | `/api/batch` | `{base: CreateRun, seeds:[...] \| count}` | `{job_id}` | planned M5 |
| GET | `/api/tuner/params?controller_id=` | none | [TunableParam](schemas/TunableParam.json)`[]` | planned M5 |
| POST | `/api/tuner/test` | none | `{ok, model, latency_ms}` | planned M5 |
| POST | `/api/tuner/suggest` | `{run_id}` | `{job_id}` | planned M5 |
| POST | `/api/tuner/auto` | auto-tune request | `{job_id}` | planned M6 |
| GET | `/api/jobs` | none | `Job[]` | planned M5 |
| GET | `/api/jobs/:id` | none | Job | planned M5 |
| POST | `/api/jobs/:id/cancel` | none | Job | planned M5 |
| WS | `/ws/runs/:id` | see WebSocket | | live |
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

### Controllers (live)

Scripts are Rhai (see `docs/CONTROLLER_CONVERSION_PROMPT.md`). Every saved version is kept. Three built-ins are seeded:
`ctl_wall_follower`, `ctl_victim_check` (wall follower plus victim identification and the blue-tile stop) and `ctl_idle`.

- `GET /api/controllers/conversion-prompt` returns `{ "prompt": "<markdown>" }` (changed from `text/markdown` in 0.3.0).
- `POST /api/controllers/validate` `{ "code": "...", "lang"?: "rhai", "robot_id"?: "rob_..." }` returns a
  [ValidationOutcome](schemas/ValidationOutcome.json) `{ ok, errors: [{line, col, message}], tunables: [TunableParam], dry_run_trace: [{t, commands, log}] }`.
  Always 200; a script with errors has `ok: false`. The dry run uses `robot_id`'s sensors (default: the reference robot).
- `POST /api/controllers` `{ "id"?, "name", "code" }` saves a new version. Without `id` a controller `ctl_<12 hex>` is
  created at version 1; with `id` the version is the previous maximum plus one (`not_found` for an unknown id).
  Returns `{ "id", "version", "tunables": [TunableParam], "validation": ValidationOutcome }`.
  `422 validation_failed` (details: the compile errors) when the code does not compile; runtime problems found by the dry
  run do not block saving, they show in `validation`. Fixture: [`controller_saved.json`](../fixtures/controller_saved.json).
- `GET /api/controllers` returns `[{ id, name, version, created_at }]`, the latest version of each.
- `GET /api/controllers/:id` returns `{ id, name, version, code, tunables, created_at }` for the latest version, or for
  `?version=N`. `not_found` for an unknown id or version.
- `GET /api/controllers/:id/versions` returns `[{ version, name, created_at }]`, newest first.

### Runs (live)

`POST /api/runs` takes [CreateRun](schemas/CreateRun.json) and returns `{ "id": "run_<12 hex>", "seed": 42, "state": "queued" }`
(fixture [`run_created.json`](../fixtures/run_created.json)). The run starts on its own thread right away.

- Resolution: the robot, maze, controller (latest version) and error profile (a preset id: `clean`, `realistic`, `chaos`)
  must exist, otherwise `404 not_found`. `rng.mode = "fixed"` needs `rng.seed` (`bad_request` without) and the seed is masked
  to 53 bits; `trng` draws a fresh seed. `422 validation_failed` when the sim cannot be built (bad maze, script does not compile).
- `speed`: 0 runs as fast as possible; otherwise sim time follows wall time times `speed`. Pacing only decides when steps run,
  so the replay does not depend on it. `speed` must be 0 or positive.
- Ending a run: a controller's `end_run` command does not end the run by itself. The run ends with `end_reason` `exited` only when the robot is on the start tile, holding `id_light` on and stationary for `exit_light_s` (rules profile); otherwise the sim emits a `note` event and keeps running. Frames carry colour readings and camera detections, and `Metrics` fills `victims_identified`, `misidentifications`, `decoy_false_positives` and `kits_used`.
- `GET /api/runs?limit=N` returns `[{ id, maze_id, controller_id, seed, state, score_total, created_at }]`, newest first.
  `score_total` is null until the run ends.
- `GET /api/runs/:id` returns [RunDetail](schemas/RunDetail.json): `{ id, state, config: CreateRun, seed, maze_id, controller_id,
  controller_version, end_reason, metrics, created_at, finished_at }`. `end_reason` (`time_up | exited | stopped | controller_fault`)
  and `metrics` are null until the run ends. `state` is `queued | running | paused | finished | stopped | failed`:
  `finished` = ended by time or exit, `stopped` = stopped by the user, `failed` = controller fault or server error.
  Fixture: [`run_detail.json`](../fixtures/run_detail.json).
- `GET /api/runs/:id/metrics` returns the [Metrics](schemas/Metrics.json); `409 conflict` until the run has ended.
- `GET /api/runs/:id/replay` returns the [Replay](schemas/Replay.json) as JSON, or MessagePack (`application/msgpack`) with
  `?format=msgpack` or `Accept: application/msgpack`. `?format=json|msgpack` wins over the header. `409 conflict` while the
  run is live. Replays of the same seed are byte identical except `header.run_id` and `header.created_at`.
- `POST /api/runs/:id/pause|resume|stop` return `{ "state" }` (`paused`, `running`, and the stored final state for stop;
  stop waits for the result to be stored, up to 5 s). `409 conflict` if the run is not live, already paused (pause), not
  paused (resume). `404` for unknown ids.
- `POST /api/runs/batch` `{ "base": CreateRun, "seeds": [1,2,3] }` (1 to 500 seeds) runs every seed headless
  (`rng` = fixed with that seed, `speed` 0), at most `available_parallelism` at a time, and answers when all are done:
  `{ "runs": [{ id, seed, state, score_total }], "summary": { count, scored, mean_score, min_score, max_score } }`.
  The runs are normal stored runs. (`POST /api/batch`, the M5 job version, is still 501.)

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

## WebSocket

Text frames, one JSON object each, discriminated by `type`. Types: [WsServerMessage](schemas/WsServerMessage.json),
[WsClientMessage](schemas/WsClientMessage.json). Example sequence: [`ws_messages.json`](../fixtures/ws_messages.json).

### `WS /ws/runs/:id`

`404` (HTTP, before the upgrade) for an unknown run. Server to client:

```json
{ "type": "hello", "run": {"...": "RunDetail"}, "maze": {"...": "MazeGraph"} }
{ "type": "frame", "frame": {"t": 1.24, "pose": {}, "wheels": [], "imu": {}, "tof": [], "color": [], "cams": [], "id_light": false} }
{ "type": "event", "t": 1.3, "event": {"type": "tile_enter", "tile": {"col": 1, "row": 0}} }
{ "type": "state", "state": "paused" }
{ "type": "dropped", "count": 12 }
{ "type": "end", "reason": "time_up", "metrics": {"...": "Metrics"} }
```

- `hello` is always first: the run as `GET /api/runs/:id` returns it and the stored maze it was created from.
- `frame.frame` is a [Frame](schemas/Frame.json). Frames stream at about 25 Hz of sim time (the replay's `frame_dt_s`), so at
  `speed` s they arrive at 25 s per wall second. For `speed` 0 the stream is throttled to at most 30 messages per second of
  wall time; ask for the replay when you need every frame.
- `event.event` is the [Event](schemas/Event.json) without `t` (its own `type` names the kind, e.g. `tile_enter`), `event.t` is
  the sim time. Every event of the run is delivered exactly once and in order.
- A connection made mid-run first receives all events so far, then the latest frame, then live traffic.
- `state` is sent when a live run pauses or resumes.
- Backpressure: the server never blocks the simulation for a slow client. If the client falls behind, queued messages are
  skipped (frames are lost for good), the missed events are re-sent from the run's event log, and a `dropped` message with the
  number of skipped messages follows. Events are never lost.
- `end` is the last message (all events before it), then the server closes the socket. Connecting to a run that already ended
  gives `hello`, `end`, close. A run that failed has `reason` `controller_fault` (a server-side failure or restart without stored
  metrics sends zeroed `metrics`).

Client to server: `{ "type": "stop" }`, `{ "type": "pause" }`, `{ "type": "resume" }`. They act like the REST calls; wrong-state
requests are ignored silently (watch `state`), unknown messages are ignored.

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
Full specification: `docs/REPLAY_FORMAT.md`. Example: [`fixtures/replay_small.json`](../fixtures/replay_small.json).

## Coordination

The frontend must not rely on anything not in this file. To change the contract, add an entry to
[`API_CHANGES_PROPOSED.md`](API_CHANGES_PROPOSED.md); the backend lead accepts, rewrites or rejects it,
then bumps the version here, adds a Changelog line and updates the fixtures in the same commit.

## Changelog

- **0.3.0** (2026-10-27): live: controllers (validate, save with versions, list, get, versions, conversion prompt; built-ins
  `ctl_wall_follower`, `ctl_victim_check`, `ctl_idle`), runs (create, list, get, pause/resume/stop, replay json/msgpack, metrics,
  `POST /api/runs/batch`) and `WS /ws/runs/:id`. Decisions: `conversion-prompt` now returns `{prompt}` instead of `text/markdown`;
  `POST /api/runs` returns `id` (not `run_id`); the WS `frame` message nests the frame under `frame` and `end` carries `reason`
  and `metrics`; added `hello` and `RunDetail` fields `maze_id`, `controller_id`, `controller_version`, `end_reason`, `created_at`,
  `finished_at`; `/api/runs/:id/replay` also honours `Accept: application/msgpack`; the WS client-to-server `resume {from_t}`
  backfill from 0.1.0 was dropped (reconnect and read the replay instead). New fixtures `run_created.json`, `run_detail.json`,
  `ws_messages.json`, `controller_saved.json`; `health.json` reports 0.3.0.
- **0.2.0** (2026-10-27): live: robot create/update/list/get (`rob_reference` seeded), maze generate (response is now `{maze, warnings, ascii}`), validate, list (summaries, not full graphs), get, put (response is `{maze, validation}`). `api_version` is 0.2.0. New fixtures `maze_generated.json`, `maze_generate_response.json`.
- **0.1.0** (2026-10-27): initial draft. Live: health, spec, robot validation. Everything else registered and returning 501.
  Decisions: validation errors use HTTP 422; `event` WebSocket messages nest the `Event` under `event` to avoid clashing
  with `Event.type`; added error code `not_implemented`.
