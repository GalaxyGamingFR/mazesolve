# SKILLS.md — Reusable skill recipes for MazeMuse agents & humans

Procedures that get repeated during the build (and after the hackathon). Each skill:
**trigger → steps → gotchas**. Agents: follow these instead of re-inventing; humans: use them
as runbooks. If a skill's steps drift from reality, fix the skill in the same PR.

---

## S1. Add a new sensor model
**Trigger:** new hardware on the robot (e.g. MPU-6050 preset, ultrasonic, thermometer for heated victims).
**Steps:**
1. Add its spec struct + `SensorKind` variant in `mm-core` (serde + schemars), with every
   physical knob (rates, ranges, noise params) — datasheet values as defaults.
2. Implement `SensorModel` trait in `mm-sim/sensors/<name>.rs`: `sample(&self, world: &WorldState, pose, rng: &mut SimRng, profile: &ErrorProfile) -> Reading`.
3. Wire the reading into `Observations` (mm-core) and into the replay frame delta (only-on-change).
4. Add golden tests: fixed world + fixed seed ⇒ exact expected readings; noise-off ⇒ ideal values.
5. Add params to `ErrorProfile` presets (clean/realistic/chaos) and to `GET /api/spec` catalog.
6. Update `docs/API.md` (schemas), fixtures, and the frontend sensor-catalog + overlay chip.
**Gotchas:** never sample sensors every physics tick — respect per-sensor rates; all randomness
through the run's seeded RNG (S3); keep `Reading` compact (delta-encoded in replay).

## S2. Add a maze feature / handle a rules change
**Trigger:** 2027 draft → final rules changes, or a new tile/obstacle type (e.g. bridges).
**Steps:**
1. Put every number in `rules_profiles/rcj_maze_2027_draft.json` (dimensions, counts, limits) —
   code reads the profile, never constants.
2. Extend `MazeGraph` types (tile feature enum / zone struct) in mm-core; bump `format_version`.
3. Generator: placement rules + legality constraints (e.g. "never on ramps", "20 cm from walls");
   validator: add proptest properties (reachability preserved, counts within bounds).
4. Sim: heightfield/surface interaction (slip? stop? score event?) + scoring hook if the rules
   award points; add typed replay Event.
5. Unit-test scoring against a hand-computed example from the rules text (§5.6).
6. Frontend: tile rendering + legend + inspector + generator form toggle.
**Gotchas:** keep 2026 profile for regression tests; ±10 % tolerance jitter must apply to new
features too; victim-placement exclusion lists are easy to forget — they're in rules §3.6.

## S3. Debug nondeterminism (replay mismatch)
**Trigger:** determinism test fails, or two same-seed runs diverge.
**Steps:**
1. Reproduce: `cargo test -p mm-sim determinism -- --nocapture`; capture both replay hashes.
2. Binary-search the divergence tick (replay diff tool: `mm-cli diff a.msgpack b.msgpack`).
3. Suspects in order: (a) HashMap/HashSet iteration in sim state → BTreeMap/sorted;
   (b) wall-clock or `Instant::now()` inside the loop → remove; (c) RNG draws outside the seeded
   stream (any `rand::thread_rng`) → inject `SimRng`; (d) float nondeterminism from parallel
   reduction order → fix iteration order (rayon only outside sim-critical sections);
   (e) async interleaving leaking into sim (channels are fine; shared mutable state is not).
4. Add the failing scenario as a regression test before fixing.
**Gotchas:** sim runs single-threaded by design — "speed it up with threads" proposals inside a
run are rejected; batch-level parallelism (many runs) is where concurrency belongs.

## S4. Capture & inspect a replay
**Steps:**
1. `POST /api/runs` (or `mm-cli run --robot r1 --maze m1 --controller c1 --seed 42 --profile chaos`).
2. `GET /api/runs/:id/replay?format=json | jq` for humans; msgpack for tools;
   `mm-cli inspect replay.msgpack --events --around lop_restart` prints an event-window summary.
3. Header first: seed, config hashes, rules profile, tolerance — mismatched hashes explain most
   "why did results change?" questions.
**Gotchas:** replays are content-addressed on disk; don't hand-edit them; format changes require
`REPLAY_FORMAT.md` version bump + migration note.

## S5. Author / tune an ErrorProfile preset
**Steps:**
1. Start from `realistic`; adjust one family at a time (ToF σ/dropout, IMU drift, wheel slip,
   camera precision/recall, color noise, tolerance %).
2. Sanity-run the reference controller on 3 seeds: `clean` should score near-perfect; if it
   doesn't, the profile broke navigability, not "realism".
3. Save as named profile via API; record intent in the profile `description` field (shown in UI).
**Gotchas:** `chaos` is for robustness demos, not for tuning A/B (use fixed seed + same profile
when comparing algorithms); dropout % interacts multiplicatively across 8 sensors — keep totals sane.

## S6. Convert a team's real controller code (first-user workflow)
**Steps:**
1. In the UI: Controller Import → copy the `CONTROLLER_CONVERSION_PROMPT`.
2. Give the prompt + the team's firmware (Arduino C++/MicroPython) to any capable AI agent.
3. Paste the returned Rhai adapter back → Validate → fix reported line errors (usually unit
   mismatches: cm↔m, deg↔rad; or blocking loops → must become per-`step()` state machines).
4. Declare tunables with `// @tunable(min,max,default) name` comments so the Muse auto-tuner can see them.
5. Dry-run in Theater with `clean` profile first (logic bugs), then `realistic` (robustness bugs).
**Gotchas:** conversion prompt is a deliverable — if a conversion fails repeatedly for a common
pattern, improve the prompt doc + examples, not just that script; blocking `delay()`-style code
must become timers in `this` state.

## S7. Run an algorithm bake-off (A/B comparison)
**Steps:**
1. Fix maze (same `maze_id`) and noise (`rng.mode=fixed`, same seed); vary only controllers/params.
2. `mm-cli batch --maze m1 --seed 7 --controllers wall_follower,frontier_bug --profiles realistic --repeats 5`
   (repeats = fresh-seed robustness sweep).
3. Compare in UI Compare Lab (ghost overlay + diff table) or `POST /api/compare`.
4. Report: score breakdown, exploration %, LoP count, exit bonus, per-seed variance.
**Gotchas:** never compare across different seeds and call it A/B; one run per (controller ×
seed) is cheap — batch it; variance across seeds is itself a headline metric ("robustness").

## S8. Extend the Muse tuning loop
**Steps:**
1. Tune the **dossier** first, not the model: failure windows, event context, param snapshot,
   token budget (~30k). `mm-cli dossier run_id` prints exactly what Muse will see.
2. Keep responses schema-locked (serde validation + one repair retry). Add new suggestion
   `target` types only with clamps registered in `GET /api/tuner/params`.
3. Auto-mode changes stay inside allowlist+min/max; every applied delta is a versioned artifact
   (controller versions / profile versions) so rollback is trivial.
4. Evaluate loop quality with the audit log: accepted-vs-rolled-back ratio should improve after
   prompt changes; keep before/after examples for the submission.
**Gotchas:** never send raw replays (too big); never apply code patches without
`allow_code_changes:true` + sandbox validation; log tokens/cost per call; API key lives in env only.

## S9. Headless performance pass
**Steps:**
1. `cargo bench -p mm-sim` — gates: ≥10× realtime/run, batch ≥50 runs per 10-min maze.
2. Profile hot spots (`cargo flamegraph -p mm-cli -- run …`): usually ToF raycasts (512 rays/frame)
   and replay serialization.
3. Fixes in order: spatial hash/BVH for wall segments, delta-encode frames, stride recording
   (events full-rate, frames 20–30 Hz), reuse allocations (`Vec::clear`, no per-tick allocs).
**Gotchas:** determinism outranks speed — any optimization that reorders sim-side computation is
rejected; WS streaming must drop-with-marker under backpressure, never slow the loop.

## S10. Prepare the judge demo (3-minute script)
**Steps:**
1. `mm-cli demo` (backend seeded pack) → frontend Demo Mode: preloaded robot, maze, 2 controllers.
2. Beat 1 — Theater: live run, ToF overlays, victim found, Identification Light event.
3. Beat 2 — Failure: `chaos` profile run LoPs at a blue tile; scrub replay to the failure.
4. Beat 3 — Muse: Suggest mode diagnoses it live; then Auto-tune runs 3 iterations on stage:
   score curve climbs, rollback shown once (proves guardrails).
5. Beat 4 — Compare Lab: tuned vs untuned ghost overlay, diff table, "same seed" chip visible.
6. Backup: pre-recorded screen capture of beats 3–4 (network/API flakiness is the #1 demo killer).
**Gotchas:** hide API keys/costs screens; rehearse with backend restarted cold (SQLite from
scratch) once; keep the fixed demo seed written on a sticky note — reproducibility is the product.

---
*Skill file conventions: add new skills as S<N>, keep each under ~40 lines, and update the
trigger line whenever the underlying tooling changes.*
