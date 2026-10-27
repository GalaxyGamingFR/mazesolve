# Replay format (v1)

A `Replay` (`mm_core::Replay`) is `{ header, frames, events }`. Everything is SI: meters,
radians, seconds. World frame: x east, y north, z up; yaw CCW from +x. Robot frame: +x
forward, +y left. Wheel order is always FL, FR, RL, RR.

## Header

| field | meaning |
|---|---|
| `format_version` | 1 |
| `run_id` | caller supplied |
| `seed`, `rng_mode` | seed that produced the run (`fixed` = caller seed, `trng` = drawn by the caller from OS entropy, < 2^53) |
| `robot_hash`, `maze_hash`, `controller_hash`, `error_profile_hash`, `rules_hash` | `config_hash` of each input: 16 hex chars, FNV-1a 64 over canonical JSON (keys sorted) |
| `applied_tolerance_frac` | rules `tolerance_frac` x error profile `tolerance_use`: the fraction by which walls (grid vertices) and wall heights were jittered in this instance |
| `physics_dt_s`, `controller_dt_s`, `frame_dt_s` | fixed steps. Default 0.001 / 0.01 / 0.04 |
| `created_at` | set by the server only, `None` from the sim |
| `backend_version` | `mm-sim` crate version |

## Frames

One `Frame` every `frame_dt_s` (stride = `round(frame_dt / controller_dt)` controller ticks,
default 4), recorded at the start of the tick, plus a final frame at run end. A frame holds
the ground-truth `pose` (yaw wrapped to (-pi, pi]), per-wheel `{cmd, actual}` surface speed
(m/s), the IMU reading, and `id_light`.

Sensor snapshots are fresh-only: `tof` contains only sensors that took a new sample since the
previous frame (their `fresh` flag is true; the zones are the latest sample). A sensor
absent from a frame has not changed. `color` and `cams` follow the same rule (empty until M4).
ToF zones are millimeters, row-major, row 0 at the top and column 0 on the left;
`65535` with status `255` means no target.

## Events

Events are full fidelity (every one, in order) with a time `t` and a `type` tag
(serde `tag = "type"`, snake_case, flattened next to `t`).

| type | fields | emitted |
|---|---|---|
| `run_start` | | t = 0 |
| `tile_enter` / `tile_exit` | `tile {col,row}` | the tile holding the robot centre changes (rule 5.4.4) |
| `collision` | `x`, `y` (contact point) | wall/obstacle contact, at most one per 0.25 s of contact |
| `checkpoint` | `tile` | first visit of a silver tile |
| `blue_stop` | `tile`, `ok` | M4 |
| `victim_identified` | `victim_id`, `reported`, `correct`, `points` | M4 |
| `victim_missed` | `victim_id` | M4 |
| `misidentification` | `reported` | M4 |
| `decoy_false_positive` | `victim_id` | M4 |
| `kit_deployed` | `victim_id` (nullable), `count`, `points` | M4 |
| `ramp_traversed` | `tile`, `up`, `points` | M4 |
| `stairs_traversed` | `tile`, `points` | M4 |
| `speed_bump_traversed` | `tile`, `points` | M4 |
| `entered_black` | `tile` | robot centre enters a black tile; followed by `lop_restart` |
| `lop_restart` | `to` (tile), `restarts` (running count) | no net displacement over `lop_min_displacement_m` within `lop_no_progress_s`, or after black. Teleport to last silver tile or start, tile centre, yaw kept |
| `controller_error` | `message` | controller `init`/`step` returned an error; run ends with `controller_fault` |
| `note` | `text` | `Commands.note` |
| `run_end` | `reason` (`time_up`, `exited`, `stopped`, `controller_fault`), `score` (`ScoreBreakdown`) | last event |

## Encodings

* MessagePack via `rmp-serde` with named fields (`to_vec_named`): maps, not arrays, so the
  layout survives field additions. `encode_msgpack` / `decode_msgpack`.
* JSON via serde (`to_json`). Floats round-trip exactly (`serde_json/float_roundtrip` is on
  in the workspace).

## Rebuilding state at time t

1. Take the latest frame with `frame.t <= t`: pose, wheel speeds, IMU, `id_light`.
2. For each ToF sensor, the latest `tof` entry at or before `t` across frames up to there.
3. Replay events with `event.t <= t` for tile occupancy, score, restarts, and so on. Between
   frames the pose is interpolated by the viewer; events are exact.

## Determinism guarantee

Same `SimInput` (maze, robot, rules, error profile, settings incl. seed, params, hashes,
run id) and a deterministic controller give a byte-identical MessagePack replay. The sim
uses a fixed timestep, no wall clock, no global RNG and no unordered iteration. All
randomness comes from `SimRng` (ChaCha8 seeded by the run seed) with independent named
sub-streams per subsystem (`world/vertex`, `world/height`, `wheel/<i>`, `collision`,
`tof/<id>`, `imu`), so adding a sensor never shifts the noise of the others. Each draw count
per sample is independent of its outcome. The OS TRNG is used only by callers (`trng_seed`)
to pick the seed, which is recorded in the header.
