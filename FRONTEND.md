# PROMPT 1 — Frontend Build Order (give this to Kimi K3.1 Pro)

You are the **frontend lead** for **MazeMuse**, a localhost-only robot simulator built for the
**Meta Global AI Developer Hackathon (Oct 26 – Nov 4, 2026)**. You own everything the user sees
and touches. A separate agent (Claude Opus) owns the Rust backend; you two integrate through
**`docs/API.md`**, which Claude authors and you consume. Work in `/frontend` of the monorepo.
Do not implement backend logic, physics, or simulation math in the frontend — you render and
control; the backend computes.

## 0. Product in one paragraph

MazeMuse simulates a RoboCup Junior Rescue Maze 2027 robot (8× VL53L8CX 64-zone ToF,
1× AS7343 14-channel spectral sensor, 2× OpenMV N6 cameras, 4× DDSM210 drive wheels,
a BNO055-class IMU) in a randomly generated, rule-accurate maze. Users place sensors on their
robot body, import their (AI-converted) controller code, run the simulation under injected
sensor/actuator noise, compare algorithms side-by-side, and let Meta's **Muse API** analyze
failures — either as printed suggestions or as a **fully automated tuning loop**. Simulation
math is 3D; the default view is a **2D top-down orthographic projection** of that 3D world.

## 1. Hard constraints

- **Stack:** Vite + React 18 + TypeScript (strict) + Tailwind CSS + shadcn/ui + Zustand +
  TanStack Query + **react-three-fiber (+drei)** for all 3D/2D rendering + Recharts for charts.
- **Localhost only.** Dev server on `:5173`, backend on `:8080` (configurable via `VITE_API_BASE`).
- **Mock-first:** from hour one, run against a mock layer (MSW or a Vite dev plugin) that
  implements the draft contract in §4 with realistic canned data (a sample robot, 3 sample
  mazes, a 90-second replay with events, a completed tuning job). The app must be fully
  clickable with the backend dead. Show a "backend offline — mock mode" banner, never a white screen.
- **Contract discipline:** the moment `docs/API.md` exists in the repo, it is the single source
  of truth. Regenerate/align your API client types from it. If you find a mismatch, do NOT
  silently fork the contract: file `docs/API_CHANGES_PROPOSED.md` with a precise diff proposal.
- **Units:** backend speaks meters/radians/seconds; UI displays cm/degrees where natural
  (rules are in cm). One conversion module, used everywhere.
- **Performance:** steady 60 fps in the Sim Theater with 8 ToF sensors × 64 zones overlay +
  path trail of ≥10k points (use instancing, line segments buffers, throttle telemetry→React
  state to ~20 Hz while R3F consumes the latest frame via refs). No layout thrash from telemetry.
- **Dark theme default**, dense-but-readable "mission control" aesthetic. Colorblind-safe
  palette (avoid red/green-only encodings — use shape/pattern too; the rules' own tile colors
  black/silver/blue/red are semantic and must appear as-is on the maze).

## 2. App map (routes)

```
/            Run Dashboard        /build/robot     Robot Builder
/mazes       Maze Studio          /build/controller Controller Import
/runs/:id    Simulation Theater   /compare         Compare Lab
/tune        Muse Tuning Lab      /settings        Settings
```

Global: top bar with app name, backend status LED (WS connected / REST reachable / offline),
active run chip (ID + elapsed + score), and a command palette (⌘K) for navigation and actions.

## 3. Screen specifications

### 3.1 Run Dashboard (`/`)
- Cards for recent runs: maze thumbnail (top-down SVG snapshot), controller name, seed chip,
  score, outcome (Exited / LoP-restarted ×n / Timed out / Crashed-stuck), sparkline of score-over-time.
- Primary CTA "New Run" → 4-step wizard modal: **Robot → Controller → Maze → Run options**
  (RNG mode: `TRNG-seeded` vs `Fixed seed` (seed field + dice button); error profile preset:
  Clean / Realistic / Chaos / Custom; sim speed: 0.5×/1×/4×/16×/max; max duration).
- Side panel: quick stats (runs today, best score, algorithms tested), and a "Demo Pack"
  button that loads pre-seeded showcase data (judges mode).

### 3.2 Robot Builder (`/build/robot`)
- Inputs: robot **length / width / height** (cm, validated against the 2027 rule: height ≤ 25 cm,
  must not exceed tile geometry sanity; show rule hints inline).
- A 3D scene of the robot chassis (box) with **drag-and-drop sensor placement**: snap to
  faces/edges, free XYZ position + yaw/pitch/roll, per-item parameter panel.
- Sensor catalog (exact set, with rule-accurate defaults):
  - `vl53l8cx` ×8 — 64-zone ToF; params: FoV (default 45°), max range 4 m, refresh rate, mount pose. Preset button "Ring layout" (4 corners ±45°, 4 faces) to one-click a typical build.
  - `as7343` ×1 — spectral; params: gain, integration time, orientation (typically down-facing for floor tiles).
  - `openmv_n6` ×2 — cameras; params: horizontal/vertical FoV, detection precision/recall sliders, latency ms.
  - `imu` ×1 — preset selector: `BNO055 (fused)` or `Raw IMU (MPU-class)`; params: heading drift °/min, accel/gyro noise σ.
  - `ddsm210` ×4 — wheels; params: radius, track/positions (left/right pairs), max speed, deadband, slip coefficient.
- Live preview: top-down gizmo showing each ToF's 45° cone, camera frustums, wheel track; a
  "coverage sanity" warning if any direction has no ToF within range.
- Validation panel (calls `POST /api/robots/validate`): overlapping sensors, out-of-bounds
  mounts, >4 wheels, height violation. Save → robot card with version number.

### 3.3 Maze Studio (`/mazes`)
- Generator form: **seed** (text + dice), grid size (tiles W×H, default per rules e.g. 8×6 up
  to 14×10), rules profile (locked to `rcj_maze_2027_draft`, shown read-only with a spec sheet
  drawer: 28 cm tiles, 12.5 cm walls, ±10 % tolerance), feature toggles + counts:
  black (hole) tiles, silver checkpoints, blue puddle tiles, red Dangerous-Zone entrance (with
  DZ on/off), ramps (≤25°), stairs (step ≤2 cm), speed bumps (≤1 cm / ≤2 cm in DZ), obstacles
  (≥15 cm), smiley victims H/S/U counts, cognitive targets count, decoy count.
- 2D top-down preview (canonical) + toggle to 3D orbit view (walls at 12.5 cm, ramps/stairs as
  wedges/steps, victims as wall decals: 🙂/😐/😟 and 5-ring cognitive targets).
- Semantic legend always visible. Click a tile → inspector (type, features, victim mount,
  linear-vs-floating classification from the generator response).
- Manual edit mode (paint tile types, toggle walls, move victims) → `PUT /api/mazes/:id`;
  re-validation chip ("start reachable to 100 % of non-black tiles: ✓"). Export maze JSON button.

### 3.4 Controller Import (`/build/controller`)
Three-pane workflow:
1. **Source pane:** paste original firmware (Arduino C++ / MicroPython), select language.
2. **Prompt pane:** shows the project's `CONTROLLER_CONVERSION_PROMPT` (fetched from backend
   `GET /api/controllers/conversion-prompt`) with copy button and short instructions: "give this
   to any AI agent together with your code, paste the result back".
3. **Result pane:** paste converted controller script; "Validate" → `POST /api/controllers/validate`
   → show compile status, sandbox dry-run trace (first 50 steps of stub observations), errors
   with line numbers; "Save" → named controller card with diff-vs-previous-version view.

### 3.5 Simulation Theater (`/runs/:id`) — the money screen
- **Main view:** 2D top-down orthographic (default) with pixel-accurate maze, robot footprint
  rectangle showing heading, and toggle to free 3D orbit camera (same scene, perspective cam).
- **Live overlays (toggle chips):** per-ToF 8×8 zone heat-grid rendered at sensor pose (opacity
  by status), ToF cones, camera frustums with detection pings, AS7343 current 14-channel
  swatch strip, path trail (colored by speed or by time), visited-tile shading (exploration %),
  victim markers (found = filled, missed-decoy = ⚠), event popups (checkpoint ✓, LoP restart ↺,
  blue-tile 5 s stop ⏱, wall collision ✳, kit deployed 📦).
- **HUD (right rail):** sim clock, real speed multiplier, score (rule table breakdown on hover),
  victims identified vs existing, restarts, exploration %, battery-less (omit), IMU strip chart
  (heading estimated vs ground-truth drift — this sells the error injection), wheel cmd vs
  encoder actual mini-charts, WS latency + FPS meters.
- **Bottom transport bar:** play/pause/step-frame, speed (0.25×–8×), **timeline scrubber** over
  the full replay with event tick marks (hover = tooltip), "jump to next failure" button.
  When scrubbing history, the sim view reconstructs from the replay (frontend never simulates).
- **Event log** drawer: filterable (sensor / navigation / scoring / error-injection / tuner),
  click event → scrub to it.
- Runs started live stream over WS; opening a finished run loads the replay and behaves
  identically (scrub from t=0, auto-play optional).

### 3.6 Compare Lab (`/compare`)
- Pick 2–4 runs (typically same maze+seed, different controllers or params).
- **Overlay view:** all paths on one maze, per-run color, toggle individual runs; "ghost"
  synchronized replay (shared transport bar; ghosts = translucent footprints).
- **Metrics diff table:** score & breakdown, victims found/missed/misidentified (incl. decoys),
  exploration %, path length & efficiency (path/ideal-coverage), wall collisions, LoP restarts,
  blue-tile stops honored, time-to-exit, exit bonus earned. Best-per-column highlighting.
- **Aligned charts:** heading-drift overlay, ToF error histogram per run, score-over-time.
- "Send to Tuning Lab" button → prefills a Muse job from the selected runs.

### 3.7 Muse Tuning Lab (`/tune`)
- Provider status card: Meta Model API endpoint, model (e.g. Muse Spark), key configured ✓/✗
  (key itself lives in backend env; frontend only shows status; `POST /api/tuner/test` button).
- **Mode A — Suggest:** pick a failed/run → `POST /api/tuner/suggest` → job progress (WS) →
  rendered report: diagnosis, root cause, ranked suggestions (each: what / why / expected
  effect / confidence), copy-report button.
- **Mode B — Auto-tune:** configure budget (max iterations, wall-clock cap), parameter
  allowlist with min/max clamps (sourced from backend `GET /api/tuner/params`), seeds policy
  (same-seed A/B + fresh-seed robustness run). Start → **iteration timeline**: each iteration =
  card {trigger failure, Muse diagnosis, applied param deltas (before→after), re-run result,
  score delta, accepted/regressed-rolled-back}. Live score-vs-iteration curve. Stop button
  (respects budget). Final summary: total improvement, net param diff vs baseline,
  "apply to controller/robot config" and "export report" actions.
- Full audit log table (every Muse call: tokens, latency, model, cost estimate).

### 3.8 Settings (`/settings`)
- Backend URL, WS reconnect policy, theme density.
- **Error Profile Editor:** presets Clean / Realistic / Chaos / Custom; per-family sliders +
  numeric inputs: ToF (σ mm, distance-dependent σ, dropout %, crosstalk, quantization 1 mm,
  max-rate), IMU (heading drift, gyro/accel noise, vibration coupling), wheels (deadband, slip
  %, backlash, latency), color sensor (channel noise, gain), cameras (precision, recall,
  latency, decoy-confusion %), global (field tolerance ±% — default 10 % per the rules;
  lighting-flash event rate). Save named profiles; profiles attach to runs.
- RNG defaults: TRNG-seeded vs fixed seed.

## 4. Draft API contract (shared with backend — Claude finalizes in `docs/API.md`)

REST (JSON, `http://localhost:8080`), errors as `{ "error": { "code", "message", "details?" } }`:

```
GET  /api/health                       → { ok, version, sim_capabilities }
GET  /api/spec                         → sensor catalog, rules profiles, tunable params
POST /api/robots                       → create/update robot config { dims, sensors[], wheels[] }
POST /api/robots/validate              → { valid, issues[] }
GET  /api/robots[/:id]
POST /api/mazes/generate               → { seed, grid:{w,h}, rules:"rcj_maze_2027_draft", features{…} } → maze
POST /api/mazes/validate               → connectivity/rule compliance report
GET/PUT /api/mazes/:id
GET  /api/controllers/conversion-prompt
POST /api/controllers/validate         → { code, lang:"rhai" } → { ok, errors[], dry_run_trace? }
POST /api/controllers                  → save versioned controller
POST /api/runs                         → { robot_id, maze_id, controller_id, rng:{mode:"trng"|"fixed",seed?},
                                           error_profile_id, speed, max_duration_s } → { run_id, seed }
GET  /api/runs[/:id]                   → run meta + metrics
POST /api/runs/:id/{pause,resume,stop}
GET  /api/runs/:id/replay              → replay blob (frames + events; MessagePack w/ JSON fallback)
GET  /api/runs/:id/metrics             → scoring breakdown per 2027 rules
POST /api/compare                      → { run_ids[] } → aligned metrics
GET  /api/tuner/params                 → tunable param allowlist w/ bounds
POST /api/tuner/test                   → Muse API connectivity check
POST /api/tuner/suggest                → { run_id } → { job_id }
POST /api/tuner/auto                   → { run_id, budget, param_bounds, seed_policy } → { job_id }
GET  /api/jobs/:id                     → { state, progress, result? }
```

WebSocket:
```
WS /ws/runs/:id     → frames @ ~20-30 Hz: { t, pose:{x,y,z,roll,pitch,yaw}, wheels:{cmd,actual}[4],
                      tof:[{sensor_id, zones:u16[64], status:u8[64]}] (only changed sensors),
                      color:{ch:f32[14]}, cam_detections:[…], imu:{euler,q,bias_est}, events:[…] }
WS /ws/jobs/:id     → { progress, iteration?, log_line?, done, result? }
```
Replay file = same frame schema + event list + header { seed, versions, config hashes }.
Frontend must tolerate: unknown fields (forward-compat), missing optional frames, WS drop →
auto-reconnect with `Last-Event-ID`-style backfill via replay fetch.

## 5. Engineering requirements

- Folder layout: `src/{app,features/{robot,maze,controller,theater,compare,tune,settings},
  components,lib/{api,ws,units,format},mocks,hooks}`; feature folders own their components/stores.
- `lib/api.ts` typed client generated-or-mirrored from API.md; Zod schemas for all WS/REST
  payloads; single `useRunStream(runId)` hook encapsulating WS + backfill + teardown.
- Zustand stores: `ui`, `robotDraft`, `mazeDraft`, `runLive`, `tuner`. No prop-drilling of telemetry.
- MSW (or Vite plugin) mock handlers for every endpoint + a synthetic replay generator
  (spiral wall-follower path with planted events) so Theater/Compare/Tune are demoable alone.
- Tests: Vitest + Testing Library for critical flows (wizard, validate-error rendering,
  scrubber math); Playwright smoke: dashboard→new run wizard→theater plays mock replay.
- Accessibility: keyboard-complete flows, aria-live for run events, focus rings, ≥4.5:1 contrast.
- Every async state has loading/empty/error UI. Every destructive action has confirm.
  Toasts for job completions. No `alert()`, no dead buttons.
- README in `/frontend`: setup, env vars, mock mode, scripts (`dev`, `build`, `test`, `lint`).

## 6. Definition of done (acceptance checklist)

- [ ] Full app clickable in mock mode with backend offline (banner shown).
- [ ] Wizard creates a run against real backend; Theater streams live frames at 60 fps.
- [ ] Replay scrub + step-frame + jump-to-failure reconstruct any past frame exactly.
- [ ] Robot Builder places all 5 sensor families with validation; Ring-layout preset works.
- [ ] Maze Studio renders generated 2027-rule mazes (28 cm tiles, 12.5 cm walls, DZ, smileys,
      cognitive targets, blue/silver/black/red tiles) in 2D and 3D; manual edits round-trip.
- [ ] Controller Import 3-pane flow validates code and shows errors with line numbers.
- [ ] Compare Lab overlays ≥2 runs with synced ghost playback and diff table.
- [ ] Tuning Lab renders suggest reports and the auto-tune iteration timeline from real jobs.
- [ ] Settings error-profile editor saves profiles that runs actually consume.
- [ ] Types align with `docs/API.md` v1; zero `any` on the API boundary; `tsc --noEmit` clean.

## 7. What NOT to do

- No simulation/physics/scoring math in the frontend (render backend truth only).
- No user accounts, no cloud features, no external CDNs at runtime (bundle everything).
- No premature 3D photorealism (no PBR assets, no shadows-heavy scenes) — clarity over beauty.
- Don't touch `/backend` or edit `docs/API.md` (propose changes via API_CHANGES_PROPOSED.md).
- Don't gold-plate before the checklist in §6 is green.
