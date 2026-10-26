# PROMPT 2 — Backend Build Order (give this to Claude Opus 5.5 — backend lead)

You are the **backend lead** for **MazeMuse**, a localhost-only robot simulator built for the
**Meta Global AI Developer Hackathon (Oct 26 – Nov 4, 2026 — you have ~8 days)**. You own the
Rust + Axum + Tokio backend: simulation core, maze generation, sensor/actuator error models,
controller import, replay recording, the REST/WS API, and the **Muse (Meta Model API)**
integration with two tuning modes. You are authorized to **spawn subagents** — do so, per §12.
A separate agent (Kimi K3.1 Pro) builds the React frontend against **`docs/API.md`, which you
author and own**. You never write frontend code; you ship the contract, fixtures, and mocks it
needs.

## 0. Product in one paragraph

MazeMuse simulates a RoboCup Junior Rescue Maze **2027**-rules robot — 8× VL53L8CX (8×8-zone
ToF), 1× AS7343 (14-channel spectral), 2× OpenMV N6 cameras (abstracted as detection models),
4× DDSM210 wheels (skid-steer), tactical-grade Inertial Sense IMX-5 or BNO055-class IMU — inside randomly generated rule-accurate
mazes, under injected measurement/actuation noise. Users compare exploration/navigation
algorithms on identical mazes *and identical noise seeds*, and a Muse-powered tuner analyzes
failures and either **suggests** improvements or **fully automatically** tunes parameters
iteration-by-iteration. Simulation math is 3D (ramps, stairs, pitch/roll matter); the frontend
renders top-down by default.

## 1. Non-negotiable engineering principles

1. **Determinism.** Fixed timestep (default: physics 1 ms, controller step 10 ms, sensor
   sampling per-sensor-rate — all configurable). Same inputs + same seed ⇒ **byte-identical**
   replay. No wall-clock, no iteration-order, no HashMap-randomness in sim paths (use
   `BTreeMap`/sorted vecs in anything touching simulation state). `f64` internally, meters/radians/seconds.
2. **RNG policy (corrected design — do not deviate):** real TRNG is used **only to seed** runs.
   At run start: pull 32 bytes from OS entropy (`getrandom`), derive a seed, then drive ALL
   noise/dropout/slip/detection randomness from a fast deterministic CSPRNG/PCG stream
   (`rand_chacha` or `fastrand`-class) keyed by that seed. The seed is recorded in the replay
   header. Two run modes: `rng.mode = "trng"` (fresh entropy seed each run) and
   `rng.mode = "fixed", seed = N` (reproducible A/B). This gives true randomness *and*
   reproducibility; never stream raw TRNG into the sim loop.
3. **Speed.** Headless ≥ **10× realtime** for a single run; batch mode ≥ 50 runs/10-min-maze on
   a dev laptop. Streaming/rendering is decoupled from the sim loop (channel-bounded, drop-with-
   marker on backpressure — never slow the sim for a slow client).
4. **Rules as data.** All 2027 field/scoring numbers live in `rules_profiles/rcj_maze_2027_draft.json`
   (+ a 2026 profile for regression), never hardcoded in logic.
5. **Contract-first.** `docs/API.md` is published at milestone M1 and versioned. Breaking
   changes require a version bump + changelog entry + notice file for the frontend agent.
6. **Security:** localhost bind only (`127.0.0.1`), CORS allowlist `http://localhost:5173`,
   API keys only via env (`META_MODEL_API_KEY` etc.), keys redacted from all logs, controller
   scripts run in a sandbox with instruction fuel + no I/O.

## 2. Workspace layout (cargo workspace + assets)

```
/backend
├── Cargo.toml                  # workspace
├── crates/
│   ├── mm-core/                # shared types: RobotConfig, SensorSpec, MazeGraph, ErrorProfile,
│   │                           #   ReplayFrame/Event, Metrics, RulesProfile (serde, schemars)
│   ├── mm-maze/                # generator + validator + scorer data (rules-driven)
│   ├── mm-sim/                 # kinematic core, sensor models, error injection, RNG, replay recorder
│   ├── mm-controller/          # Controller trait, Rhai host, import validation, conversion prompt asset
│   ├── mm-tuner/               # Muse/Meta Model API client, dossier compressor, suggest + auto-tune loop
│   ├── mm-server/              # axum REST + WS, SQLite storage, job queue
│   └── mm-cli/                 # headless: run/batch/bench/export (used by tuner + demos)
├── rules_profiles/rcj_maze_2027_draft.json
├── docs/  (API.md, REPLAY_FORMAT.md, CONTROLLER_CONVERSION_PROMPT.md, TUNER_GUARDRAILS.md)
└── fixtures/ (sample robot/maze/replay JSON for frontend mocks)
```
Crates depend one-way: `core ← maze/sim/controller/tuner ← server/cli`. Keep `mm-core` free of I/O.

## 3. Simulation core (mm-sim)

- **Kinematics, NOT rigid-body physics (MVP).** Robot = 3D pose (x,y,z,roll,pitch,yaw) driven
  by 4-wheel skid-steer: per-wheel commanded velocity → DDSM210 servo model → ground-contact
  solve on the maze heightfield (tile floors, ramps ≤25°, stairs steps ≤2 cm, 3 mm tile lip,
  speed bumps as small profiled rises). Collision: swept circle/AABB vs wall segments +
  obstacle footprints; collisions produce events + impulse-free slide response + small heading
  perturbation (configurable). Rapier3D is an explicit NON-goal for v1 (leave a `PhysicsBackend`
  trait seam).
- **Step pipeline (per tick):** read commands → wheel model (deadband, accel/torque limit
  0.25 N·m rated / 0.85 stall, encoder quantization, slip μ per surface: normal / blue-tile
  (sticky: enforce the rules' 5 s stop as controller-visible "motion penalty" zone + wheel slip),
  ramp (gravity component), bumps) → integrate pose → collisions → sensor sampling (each at its
  own rate) → error injection → observation assembly → controller.step(obs) → commands → event
  detection (checkpoints, victims in view, tile entry/exit, LoP stall, exit) → replay record.
- **Lack of Progress (LoP):** no net displacement > threshold for N s (rules-conform, default
  30 s) ⇒ event `lop_restart`: teleport to last visited checkpoint (or start), increment
  restart counter (affects reliability bonus). Configurable off/on.
- **Game/session state:** run duration cap (default 8 min per rules; make configurable),
  rescue-kit inventory + deployment events, Identification Light state (on ≥5 s while still at
  victim ⇒ valid identification per 2027 rules), exit-bonus evaluation on return to start tile.

## 4. Sensor & actuator models (mm-sim) — all parameters exposed via `ErrorProfile`

1. **VL53L8CX ×8** — per sensor: 64 rays (8×8 grid) distributed in a 45° FoV cone at mount pose;
   raycast vs wall segments/obstacles/victim geometry + heightfield. Outputs: per-zone range
   (mm, quantized 1 mm, clipped 4 m, `0xFFFF`-style out-of-range sentinel) + per-zone status
   (OK / sigma-fail / wrap / no-target). Errors: σ base + distance-dependent σ, dropout %,
   ghost/crosstalk events at close range, sunlight-blinding rate (per rules' lighting warning),
   sampling rate (default 15 Hz @8×8, configurable to 60).
2. **AS7343 ×1** — material→reflectance-spectrum table (floor: normal/black/silver-reflective/
   blue/red; wall; obstacle) sampled at sensor position/orientation → 14 channel responses
   (11 VIS + NIR + Clear + Flicker) with gain & integration-time params, per-channel noise σ,
   ambient-light multiplier, optional flicker artifact. This is how controllers read tile colors.
3. **OpenMV N6 ×2** — **detection-event model, no rendering.** Per camera: frustum test vs
   victims (smiley H/S/U Ø4 cm @ ~7 cm height; cognitive targets: read the 5-ring color sum),
   decoys, and Identification-relevant objects. Outputs `CamDetection { kind, class, confidence,
   bbox_approx, distance, t }`. Errors: precision/recall sliders, decoy-confusion %, latency ms,
   motion-blur penalty when ω > threshold, lighting-flash false-negative events. Cognitive-target
   ring misreads modeled as per-ring color confusion probability.
4. **IMU — preset-driven** (ground truth pose → reported euler/quaternion): presets
   `imx5_tactical` (Inertial Sense IMX-5: tactical-grade, gyro in-run bias stability ~1.5 °/hr
   class, dynamic heading error ~0.13°, 1 kHz output — near-zero drift; GNSS/RTK unavailable
   indoors, so simulate IMU-only), `bno055` (consumer fused output, moderate heading drift),
   `raw_imu` (MPU-class raw accel/gyro: high noise, controller must fuse — tests their code
   honestly). Error mechanisms, scaled per preset: heading drift (°/hr random walk), gyro/accel
   white noise, bias instability, vibration-coupling spikes during bumps/stairs/collisions, and
   magnetic-interference events (rules: under-floor wiring!) that briefly corrupt heading —
   the dominant heading error source for the tactical preset.
5. **DDSM210 ×4** — as in §3 wheel model; report encoder ticks back to controller.
6. **Field tolerance** — global knob (default ±10 % per 2027 rules): per-run jitter of tile size,
   wall positions/heights, doorway widths — the maze the sim runs in is a *perturbed instance*
   of the generated one. This is a headline feature; make it prominent in the replay header.

Error presets: `clean` (all zero), `realistic` (datasheet-typical), `chaos` (stress), `custom`.

## 5. Maze generator + scoring (mm-maze)

- Read the official 2027 draft: https://rescue.rcj.cloud/events/2027/Rules/RCJRescueMaze2027-draft.pdf
  (fetch it; §3 Field and §5.6 Scoring are the source of truth; also skim §5 Play for LoP/exit rules).
- **Generator:** seeded, grid-of-tiles (28 cm). Produce: perimeter walls; internal wall graph
  guaranteeing start-tile connectivity to all non-black tiles; tile classification (linear vs
  floating vs start); placement of black holes, silver checkpoints, blue puddles, red
  Dangerous-Zone entrance + fully-walled DZ sub-area (DZ never required for full coverage, start
  never inside), ramps (straight, ≤25°, no peak-after-rise), stairs (between walls, step ≤2 cm,
  tread ≥15 cm, <25° / <30° in DZ), speed bumps (≤1 cm; ≤2 cm in DZ; never on ramps/stairs
  outside DZ), obstacles (≥15 cm, placement clearance rules), victims: smiley (H/S/U) +
  cognitive targets on legal walls (never facing black/silver/blue/red tiles, obstacle/bump/
  stairs/ramp tiles; ~7 cm height; some rotated), decoys. Distribution knobs for counts and
  for victim placement near linear walls vs floating (drives the 5/10/15/30 scoring tiers).
- **Validator:** property-based (proptest): solvability (≥X % reachable coverage), victim
  placement legality, DZ optionality, feature-count bounds, dimension tolerances.
- **Scoring engine (full 2027 table — this is a headline feature):** per-victim 5/10/15/30 by
  location tier; kit-deploy bonus 30 (harmed) / 10 (stable) / 0 (unharmed) with kit-rest-near-
  victim check; checkpoint 10; ramp 10 up + 10 down; stairs 10/set; speed-bump tile 5; exit
  bonus: 10/identified victim + 10 per blue tile visited exactly once + 5/stairs + 5/ramp, only
  when run ends on start tile; misidentification penalty per rules; reliability (restart count)
  reporting. Encode §5.6 exactly after reading it; unit-test against hand-computed examples.
- **Output format:** `MazeGraph` JSON: tiles[] {type, features[], height_ref}, walls[]
  {segment x1,y1,x2,y2,h}, victims[] {kind, class|rings, pose, tile, wall_face}, zones
  {start, dangerous_zone, checkpoints[]}, header {rules_profile, seed, tolerance}.

## 6. Controller import (mm-controller)

- **Trait:** `trait Controller { fn init(&mut self, ctx: &RobotInfo); fn step(&mut self, obs: &Observations) -> Commands; }`
  where `Observations` mirrors the sensor outputs in §4 (ToF zones, spectral channels, cam
  detections, IMU, encoders, timers) and `Commands` = per-wheel velocity targets + kit-deploy +
  identification-light + (optional) notes.
- **Execution host: Rhai** (pure-Rust, sandboxable). Settings: instruction fuel per step,
  step timeout, no file/net/stdlib access, deterministic float mode. Compiled script stored
  versioned in SQLite. (WASM host = documented future seam; not in MVP.)
- **Conversion prompt asset (`docs/CONTROLLER_CONVERSION_PROMPT.md` — you author this, it is a
  graded deliverable):** a copy-pasteable prompt the user gives to ANY AI agent along with their
  Arduino-C++/MicroPython firmware. It must specify: the Rhai adapter contract, the full
  `obs`/`cmds` API table with units, mapping guidance (e.g. `vl53l8.read()` → `obs.tof[i].zones`,
  IMU heading → `obs.imu.yaw`, motor writes → `cmds.wheel_v[i]`), state-persistence patterns
  (Rhai `this` object), the 5 s blue-tile rule, LoP button semantics, and **two complete worked
  examples** (a wall-follower and a victim-check stub). Goal: converted scripts pass validation
  without human edits in ≥80 % of typical cases.
- **Validation endpoint:** compile → dry-run 50 steps against stub observations → return
  structured errors (line/col) + trace. Reject on fuel exhaustion/panic.

## 7. Replay format (`docs/REPLAY_FORMAT.md`)

Header: { format_version, run_id, seed, rng_mode, config hashes (robot/maze/controller/
error_profile/rules), created_at, backend_version }. Body: frame list (pose, wheel cmd/actual,
IMU reported, changed-sensor snapshots with per-zone u16+status delta-compressed, color
channels, cam detections) + event list (typed: collision, checkpoint, victim_identified,
victim_missed, decoy_false_positive, kit_deployed, lop_restart, blue_stop, tile_enter/exit,
ramp/stairs/bump traversed, run_end with scoring breakdown). Encoding: MessagePack primary,
JSON export endpoint for debugging/frontend fallback. Frames ~20–30 Hz (sim ticks stay 1 kHz —
record at stride, events at full fidelity). Must reconstruct exact visual state at any t.

## 8. Server (mm-server) — REST + WS + storage

- Implement the draft contract below, then **finalize `docs/API.md`** with: every endpoint
  (method, path, request/response JSON Schema, examples, error codes), both WS channel message
  schemas with field tables, replay format summary, auth notes (none; localhost), and a
  changelog. Include a **frontend fixtures pack** (`/fixtures`) matching API.md so Kimi's mock
  layer can be diffed against reality.
- Draft contract (align with frontend prompt; you may refine names — API.md wins):

```
GET  /api/health · GET /api/spec
POST /api/robots · POST /api/robots/validate · GET /api/robots[/:id]
POST /api/mazes/generate · POST /api/mazes/validate · GET/PUT /api/mazes/:id · GET /api/mazes
GET  /api/controllers/conversion-prompt · POST /api/controllers/validate · POST /api/controllers
POST /api/runs · GET /api/runs[/:id] · POST /api/runs/:id/{pause,resume,stop}
GET  /api/runs/:id/replay?format=msgpack|json · GET /api/runs/:id/metrics
POST /api/compare {run_ids[]}
GET  /api/tuner/params · POST /api/tuner/test · POST /api/tuner/suggest · POST /api/tuner/auto
GET  /api/jobs/:id · GET /api/jobs (list)
WS   /ws/runs/:id   (live frames+events; resume=backfill-from-replay)
WS   /ws/jobs/:id   (tuner job progress)
```

- Storage: SQLite (sqlx or rusqlite): robots, mazes, controllers (versions), runs (meta+metrics),
  jobs, tuner audit log. Replays on disk (content-addressed), DB stores path+hash.
- Run execution: tokio task per run, bounded mpsc to WS broadcaster, pause/resume/stop control
  channel, graceful shutdown persists partial replay. Batch endpoint or CLI for N runs
  (seed sweep) feeding the tuner.

## 9. Muse integration (mm-tuner) — the hackathon's core requirement

- **Provider adapter** for the **Meta Model API** (dev.meta.ai/docs — read the quickstart &
  models pages first; per LiteLLM/promptfoo it is OpenAI-compatible: base URL + bearer key,
  chat completions, Muse Spark supports reasoning effort, tool calling, structured outputs).
  Config via env: `META_MODEL_API_KEY`, `MUSE_BASE_URL`, `MUSE_MODEL` (default: newest Muse
  Spark id per docs), `MUSE_REASONING_EFFORT`. Keep the adapter trait so another OpenAI-compatible
  provider can be dropped in for local testing without a key.
- **Dossier compressor:** replay+metrics → compact failure dossier (≤ ~30k tokens): run config
  summary, score breakdown, event timeline focused on failures (LoP, collisions clusters,
  missed victims/decoy FPs, exit-bonus misses), sensor error stats around failure windows,
  current tunable param snapshot, controller code excerpt (relevant handler only).
- **Structured responses:** request JSON-schema-constrained output:
  `{ diagnosis, root_cause, evidence[], suggestions[] { title, detail, target(param_path|code),
  proposed_change, expected_effect, confidence }, }` — via tool-calling/JSON mode; validate with
  serde; on parse failure retry once with a repair prompt.
- **Mode A — Suggest:** one-shot: dossier → report (persisted job). No side effects.
- **Mode B — Fully automated tuning loop:** per iteration: run (same seed as baseline for A/B,
  plus fresh-seed robustness run) → analyze failure → propose **param deltas within the allowlist
  and clamps** from `GET /api/tuner/params` (error-profile robustness settings, controller-exposed
  tunables like thresholds/gains/timeouts declared in a `@tunable` comment convention in Rhai) →
  apply → re-run → accept if score/robustness improves else **rollback**. Guardrails:
  max_iterations, wall-clock budget, per-param change-rate limits, no-regression rollback,
  full audit log (every prompt/response/delta/score — exportable for the demo + submission).
  Code-level patches: Muse MAY propose Rhai diffs — v1 applies them only behind an explicit
  `allow_code_changes: true` flag, validated + sandboxed, each kept as a new controller version.
- **Ops:** retry w/ exponential backoff + jitter, timeout, rate-limit header respect, token/cost
  accounting per job, keys never logged, jobs cancellable, progress streamed on `/ws/jobs/:id`.
- Write `docs/TUNER_GUARDRAILS.md` documenting allowlist, clamps, rollback policy.

## 10. Testing & quality bar

- Unit: each sensor model (golden-value tests), scoring engine (hand-computed 2027 examples,
  incl. exit-bonus and blue-tile edge cases), maze generator **proptest** properties (§5),
  dossier compressor token bounds, tuner JSON-schema validation + rollback logic (mock provider).
- Integration: **determinism test** — same seed run twice ⇒ identical replay bytes; API contract
  tests asserting responses match the schemas embedded in API.md (generate OpenAPI from
  `utoipa`/schemars and diff); WS resume-after-drop test.
- Perf: criterion benches — sim ticks/s, 64-ray ToF × 8 sampling cost, replay size per minute;
  CI fails if single-run headless speed < 10× realtime.
- `cargo fmt` + `cargo clippy -- -D warnings` clean; `thiserror` in libs / `anyhow` in binaries;
  no `unwrap()` on request paths; tracing spans for run/job lifecycles.

## 11. Milestones (8 days)

- **M1 (D2):** workspace, mm-core types, rules JSON, **docs/API.md v0.1 (draft contract frozen
  enough for frontend)**, /api/health, /api/spec, fixtures pack. ⇐ frontend unblocks here.
- **M2 (D3):** maze generator 2027 features + validator + property tests; robot CRUD + validate.
- **M3 (D4):** sim core kinematics + heightfield + collisions; ToF & IMU & wheel models;
  ErrorProfile; replay recorder + REPLAY_FORMAT.md.
- **M4 (D5):** remaining sensors (AS7343, cameras, tolerance jitter); Rhai controller host +
  conversion prompt doc + validation; scoring engine + LoP + exit bonus; live WS run streaming.
- **M5 (D6):** Muse client + dossier + Mode A suggest; jobs + WS progress; compare endpoint; CLI batch.
- **M6 (D7):** Mode B auto-tune loop + guardrails + audit log; perf pass; determinism & contract
  tests green; end-to-end demo script (`mm-cli demo` reproducing: fail → diagnose → auto-fix → improve).
- **D8:** freeze, docs polish, seeded demo pack (robot + maze + 4 runs + 1 tuning job) for the frontend's Demo Mode.

## 12. Subagent orchestration (you are the lead)

Spawn subagents for parallelizable, well-specified units AFTER M1 (types+contract exist first —
that's the shared ground truth): `maze-agent` (mm-maze + rules JSON + proptests),
`sensors-agent` (mm-sim sensor models + golden tests), `simcore-agent` (kinematics/heightfield/
collisions/scoring), `controller-agent` (Rhai host + conversion prompt + examples),
`tuner-agent` (Muse adapter + dossier + modes), `server-agent` (axum/SQLite/WS),
`qa-agent` (determinism/contract/perf tests). You personally own: mm-core, API.md, milestone
integration, and final review of every subagent PR (clippy+tests+contract conformance before merge).

## 13. Boundaries & coordination

- Never modify `/frontend`; ship fixtures instead. API changes ⇒ API.md version bump +
  `docs/API_CHANGES_PROPOSED.md` review + notify in coordination log. Read the repo's
  `AGENTS.md`, `CLAUDE.md`, `SKILLS.md` and follow them.
- Deliverables checklist: working backend (all endpoints), `docs/API.md`,
  `docs/REPLAY_FORMAT.md`, `docs/CONTROLLER_CONVERSION_PROMPT.md`, `docs/TUNER_GUARDRAILS.md`,
  fixtures pack, demo pack, tests green, `/backend/README.md` (setup, env vars, run commands).
