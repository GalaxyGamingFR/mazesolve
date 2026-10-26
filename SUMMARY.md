# MazeMuse — Research Summary & Hackathon Verdict

> Codename **MazeMuse** (maze + Muse API). Rename freely.
> Prepared 2026-10-27 for the Meta Global AI Developer Hackathon (Oct 26 – Nov 4, 2026).
> Scope: a localhost robot simulator for a RoboCup Junior Rescue Maze robot
> (8× VL53L8CX ToF, 1× AS7343 spectral, 2× OpenMV N6 cameras, 4× DDSM210 wheels,
Inertial Sense IMX-5 tactical-grade / BNO055-class IMU),
> with random maze generation, error injection, algorithm comparison, and a Muse-API-powered
> suggest / auto-tune loop. Backend: Rust + Axum + Tokio (Claude Opus). Frontend: Kimi K3.1 Pro.

---

## 1. What the research says

### 1.1 The hackathon is real and it is NOW
- The **Meta Global AI Developer Hackathon** runs **online Oct 26 → Nov 4, 2026** (ten days), $1M in prizes, free entry if selected.
  Sources: https://cerebralvalley.ai/e/meta-ai-developer-hackathon, https://www.instagram.com/p/DexCBjIHROY/, https://developers.meta.com/blog/meta-connect-recap/
- You build with Meta's newest models: **Muse Spark** (reasoning/coding LLM), **Muse Wonder**, **Muse Image**, **SAM 3.1**, served through the **Meta Model API** ($150 credits per participant; partners Fireworks AI / Baseten / Modal for open-weights).
- **Implication:** you have ~8 days left, not "someday". Scope discipline is the whole game. Also: the Muse loop in your idea isn't a gimmick — it's *the required integration* for judging, which makes your concept strongly on-theme.

### 1.2 The "Muse API" is documented and OpenAI-compatible
- Docs live at **https://dev.meta.ai/docs** (overview, quickstart, models). Six model lines; Muse Spark supports **reasoning effort, multimodal input, tool calling** (per https://www.promptfoo.dev/docs/providers/meta/).
- **LiteLLM ships a Meta provider** and its example uses `from openai import ...` — i.e. the Meta Model API speaks the OpenAI chat-completions dialect (https://docs.litellm.ai/docs/providers/meta).
- **Implication:** backend integration = base URL + API key + model name in env vars, an OpenAI-compatible client, retries/backoff. Low risk. Claude must read `dev.meta.ai/docs/quickstart` to pin exact model IDs and endpoint shape.

### 1.3 RoboCup Junior Rescue Maze **2027 draft rules already exist**
Published Oct 2026: **https://rescue.rcj.cloud/events/2027/Rules/RCJRescueMaze2027-draft.pdf**
(forum thread: "2027 RCJ Rescue Maze Draft Rules Published – Feedback Wanted"). Key changes vs 2026 and the numbers the maze generator must encode:

| Item | 2027 spec |
|---|---|
| Tile | **28 cm × 28 cm** (was 30×30) |
| Wall | **≥ 12.5 cm high** (was 15), tile-length, mounted on tile edges; path between opposite walls = 28 cm |
| Tolerance | **±10 % on ALL measurements** (great for the error-injection story!) |
| Black tiles | holes — no-go, act as virtual walls for linear/floating classification |
| Silver tiles | checkpoints (reflective floor), 10 pts each; LoP restart point |
| **Blue tiles** | puddles — robot must **stop 5 s** before visiting another tile; exit bonus for blue tiles visited exactly once |
| **Red tile** | entrance of the **Dangerous Zone** (walled-off harder area; never required for full coverage; start tile never inside) |
| Speed bumps | ≤ 1 cm (≤ 2 cm in DZ), never on ramps/stairs outside DZ |
| Stairs | step ≤ 2 cm, tread ≥ 15 cm, incline < 25° (< 30° in DZ), width = path |
| Ramps | ≤ 25°, always straight, no peak structures |
| Obstacles | ≥ 15 cm high, any shape; 20 cm-from-wall placement rules; may be knocked and stay |
| Bridges | tiles above tiles, ≥ 25 cm clearance (stretch goal for sim) |
| Victims | **Smiley victims** (emoji faces on walls, Ø 4 cm, ~7 cm above floor; sad=Harmed, neutral=Stable, happy=Unharmed) **replacing 2026 letters**, plus **cognitive targets** (5 concentric rings Ø1–5 cm, colors Black=−2, Red=−1, Yellow=0, Green=+1, Blue=+2; sum → status: 2=H, 1=S, 0=U) |
| Decoys | objects resembling victims that are NOT victims |
| Rescue kits | ≥ 1.5 cm each dimension, ≥ 3.375 cm³ after deployment |
| Scoring | 5/10/15/30 pts per victim by location; kit bonus 30 (harmed) / 10 (stable) / 0 (unharmed); checkpoint 10; ramp 10 up/down; stairs 10; speed bump tile 5; exit bonus: 10/identified victim + 10/once-only blue tile + 5/stairs + 5/ramp; Identification Light must stay on ≥ 5 s while still |
| Robot | ≤ 25 cm high, must not see over walls, fully autonomous, no pre-mapped dead reckoning |

Exact scoring table (rule §5.6) sits in later pages of the PDF — PROMPT2 instructs Claude to fetch the PDF and encode §5.6 verbatim.

### 1.4 Your sensor stack checks out (and matters for judging)
| Part | Reality | Sim relevance |
|---|---|---|
| **VL53L8CX** (ST) | 8×8 = 64-zone ToF, up to **4 m**, 1 mm resolution, ~45° FoV, up to 60 Hz (rate depends on resolution), per-zone status flags. https://www.pololu.com/product/3419, https://www.st.com/resource/en/datasheet/vl53l8cx.pdf | Model as 64 raycasts in a cone per sensor + quantization, noise, dropout, crosstalk, status. 8 sensors × 64 zones = 512 rays/frame — trivial for Rust raycasting against wall segments. |
| **AS7343** (ams OSRAM) | **14-channel** spectral (11 VIS + NIR + Clear + Flicker), 6 parallel ADCs. https://ams-osram.com/products/sensor-solutions/ambient-light-color-spectral-proximity-sensors/ams-as7343-spectral-sensor | Model as material→spectrum lookup (black/silver/blue/red/tile colors) → 14 channel responses + gain/integration-time/noise. This is exactly how the robot will read tile colors in-maze. |
| **OpenMV N6** | AI-capable camera MCU board, MicroPython ML vision. https://openmv.io/products/openmv-n6 | Don't render photorealistic frames. Model as *detection events* (smiley/cognitive-target/decoy) with configurable precision/recall/latency — that's what the robot's code actually consumes. |
| **DDSM210** | Direct-drive serial bus servo motor w/ encoder, 11–28 V, 0.25 N·m rated / 0.85 N·m stall. https://www.waveshare.com/wiki/DDSM210 | Velocity-servo wheel model: deadband, accel/torque limits, encoder quantization, slip on blue tiles/ramps. 4 wheels = skid-steer kinematics. |
| **BNO055** (Bosch) | 9-axis **absolute orientation** sensor, on-chip fusion (Euler/quaternion output). https://www.bosch-sensortec.com/media/boschsensortec/downloads/datasheets/bst-bno055-ds000.pdf | Fused-output IMU model: heading drift, bias random walk, vibration coupling during bump/stair events. |
| **IMX-5** (Inertial Sense) | **Tactical-grade** 10-DOF IMU/INS module: gyro in-run bias stability in the 0.5–5 °/hr tactical band (~1.5 °/hr for this class), dynamic roll/pitch ~0.04°, dynamic heading ~0.13°, 1 kHz data rate, RTK-GNSS option. https://inertialsense.com/imx-5-tactical-grade-imu/ , https://inertialsense.com/tactical-imu-defense/ | Near-zero-drift IMU preset (`imx5_tactical`). Two sim consequences: ① GNSS is useless indoors → model IMU-only output; heading errors come from vibration + the rules' own magnetic-interference warning, not bias drift. ② Dead reckoning quality becomes a first-class variable — an IMX-5 vs BNO055 A/B on the same algorithm is a great demo of why error injection matters. |

### 1.5 Prior art — you are not first, but you ARE different
- **Erebus** (https://github.com/robocup-junior/erebus) is the *official* RCJ Rescue **Simulation sub-league** environment, built on **Webots** — robots explore a virtual maze and identify victims. There is even a simulation league (https://junior.robocup.org/rcj-rescue-sim/).
- General simulators (Webots, Gazebo, CoppeliaSim) can build maze worlds but: heavyweight, slow iteration, no VL53L8CX 64-zone model out of the box, no RoboCup-2027-rule maze generator, and **nothing** that ingests a team's own firmware and auto-tunes it with an LLM.
- **Your differentiators (judge-facing):** ① 2027-rules-accurate random maze generator, ② faithful model of *this exact sensor suite* incl. error injection under the rules' own ±10 % tolerance, ③ the LLM conversion-prompt pipeline that imports real team code, ④ the **Muse-powered failure-diagnosis + fully-automated tuning loop**, ⑤ head-to-head algorithm comparison on identical mazes AND identical noise seeds.
- Framing tip for the submission: don't pitch "another simulator" — pitch **"an AI engineer-in-the-loop that makes RoboCup Rescue teams better"**, with the simulator as its instrument.

---

## 2. Verdict: is this a good idea? Is it viable?

**Short answer: Yes — unusually well-aligned with this specific hackathon — but only if you cut scope hard and fix two design flaws.**

Scorecard (out of 10):

| Dimension | Score | Why |
|---|---|---|
| Hackathon fit (uses Muse API meaningfully) | **9.5** | The suggest/auto-tune loop is a native Muse Spark use case (reasoning + tool calling + structured JSON). Judges see their models doing real work. |
| Originality vs prior art | **7** | Erebus exists, but nobody does code-import + AI auto-tuning + 2027-rule generation for this exact hardware. |
| Real user value (RCJ teams) | **9** | Teams desperately want cheap at-home practice + algorithm A/B testing; mazes change right before runs, so sim practice is legit prep. |
| Demo-ability in 3 minutes | **9** | Top-down replay, side-by-side ghost runs, and a live "Muse found the bug → auto-fixed the param → score went up" moment is demo gold. |
| Feasibility in 8 days, 2 humans + 2 AI agents | **5.5** | The single biggest risk. Viable only with the MVP cuts below. |
| Technical risk (physics/sensors) | **7 (after fixes)** | Kinematic 3D sim + raycast ToF is very doable in Rust. Don't attempt full rigid-body physics. |

### The GOOD
1. **Right stack.** Rust + Axum + Tokio + fixed-timestep deterministic core = fast headless batch runs (dozens of mazes/algos/seeds per minute) — that's what makes AI auto-tuning loops practical at all. Python simulators can't iterate this fast.
2. **"3D math, 2D view" is correct.** Rules are 3D (ramps ≤25°, stairs, bridges, victims 7 cm up, walls 12.5 cm, robot ≤25 cm), but teams think top-down. Orthographic top-down default + optional 3D orbit camera is the right UX.
3. **Error injection is your moat.** The rules *themselves* mandate ±10 % tolerance, magnetic interference, lighting flashes, textured floors. Simulating ToF noise/dropout, IMU drift, wheel slip, camera false-positives is exactly the gap between Erebus-grade sim and reality.
4. **The conversion-prompt idea is smart** — but see flaw #2 below for how to make it actually work.
5. **Localhost-only for v1 is right**: zero auth, zero deploy, low-latency WebSocket telemetry, and it fits the hackathon demo story. (Server ideas later, as you said.)

### The BAD / RISKS (and the fixes)
1. **🔴 FLAW: "Completely random TRNG" breaks your own core feature.**
   You want to *compare algorithms on the same maze* — and to let Muse *diagnose a failure and re-test the fix*. Both require **reproducibility**. If every run uses fresh true randomness, A/B comparisons are noise, and the auto-tune loop can never tell whether a change helped or the dice just rolled better. Also, hardware TRNG throughput is low; nobody streams TRNG into a 1 kHz physics loop.
   **Fix (industry standard):** seed from the OS TRNG (`getrandom`) *once per run*, then run a fast deterministic CSPRNG/PCG stream from that seed. Record the seed in the replay header. Offer two modes in the UI: **TRNG-seeded** ("fresh chaos every run") and **fixed seed** ("identical maze + identical noise realization" — the fair A/B mode). You get true randomness *and* replayability. PROMPT2 mandates this.
2. **🔴 FLAW: "User pastes code → their AI converts it → paste back" needs a hard target contract.**
   Free-form converted code can't be loaded safely or consistently. **Fix:** define one tiny adapter contract — a controller script (Rhai, a pure-Rust embeddable language: `fn init()`, `fn step(obs) -> cmds`) — and ship a **conversion prompt** (`docs/CONTROLLER_CONVERSION_PROMPT.md`) that tells any AI agent exactly how to translate Arduino C++/MicroPython firmware into that contract, including the full `obs`/`cmds` API table and 2 worked examples. Backend validates on import (compile + dry-run + instruction-fuel sandbox). This keeps import robust, safe, and recompilation-free. A native-Rust trait path can exist later for power users.
3. **🟠 Scope explosion.** Full 3D rigid-body physics + photorealistic cameras + 8 ToF + spectral + maze features + scoring + import + tuner + two UIs ≈ a startup, not 8 days.
   **Fix — MVP cuts (already baked into the prompts):**
   - **Kinematic** skid-steer core (circle/swept-AABB vs wall-segment collision), NOT a rigid-body engine. Rapier3D is a post-hackathon option for knockable obstacles.
   - Cameras = **detection-event models** (precision/recall/latency/decoy confusion), not rendered frames.
   - Mazes = grid-graph generator with the 2027 feature set, but **bridges = stretch**; obstacle physics = static blockers.
   - Scoring engine: full 2027 table incl. LoP restarts, blue-tile 5 s stop, exit bonus — this one is NOT cut, because it's your headline metric.
   - Auto-tune v1 = **parameter tuning** (error-profile robustness, controller gains/thresholds exposed as tunables) + *suggested* code patches shown as diffs; full autonomous code rewriting = stretch.
4. **🟠 Muse latency/context limits.** Full replays are far too big for one prompt. **Fix:** the tuner sends a *compressed failure dossier* (metrics, event timeline around failures, param snapshot, last-N sensor summaries), demands **schema-constrained JSON** back (diagnosis → root cause → clamped param changes), retries with backoff. Auto mode has guardrails: param allowlist + min/max clamps, max iterations, rollback-on-regression, full audit log.
5. **🟠 Two-AI-agent integration risk.** Kimi and Claude working against an imagined API = integration hell on day 7. **Fix:** contract-first. PROMPT2 makes Claude ship **`docs/API.md`** (REST + WebSocket schemas + examples + error codes) as an early, versioned deliverable; both prompts embed the *same draft contract* so work can start in parallel; Kimi builds against a mock server that implements the draft; any contract change goes through API.md (AGENTS.md defines the protocol).
6. **🟡 Sim-to-real gap.** A tuned-in-sim controller can still fail on a real field (lighting, carpet, magnetic noise). **Fix honestly:** position the tool as *comparative* ("algorithm A beats B under noise profile X") not *predictive*; the ±10 % tolerance knob and "Chaos" error presets make robustness testing the selling point.
7. **🟡 Rules are DRAFT.** 2027 specs may shift. **Fix:** rules live in a data file (`rules_profile_2027.json`), never hardcoded — a rules change is a config edit. (Also note: your app is a training tool, not an official Erebus replacement; say so to avoid "unofficial simulator" confusion.)

---

## 3. Recommended architecture (as encoded in PROMPT1/PROMPT2)

```
┌────────────────────────────── localhost ──────────────────────────────┐
│  Frontend (Kimi)                     Backend (Claude Opus)            │
│  React+TS+Vite                       Rust workspace (Axum+Tokio)      │
│  react-three-fiber                   ├─ core      (types, rules JSON) │
│  · Robot Builder (sensor placement)  ├─ maze      (2027 generator)    │
│  · Maze Studio (seeded gen+edit)     ├─ sim       (kinematic 3D core, │
│  · Sim Theater (2D top-down default, │ │            sensors, errors)  │
│    3D orbit optional, replay scrub)  ├─ controller(Rhai host+import)  │
│  · Compare Lab (ghost A/B overlays)  ├─ tuner     (Muse/Meta API)     │
│  · Muse Tuning Lab                   └─ server    (REST+WS, SQLite)   │
│  · mock mode (msw) ──────────────────── REST :8080 / WS ──────────────┤
│         ▲  docs/API.md = single source of truth (Claude owns)         │
└────────────────────────────────────────────────────────────────────────┘
                 │ run replay + metrics dossier
                 ▼
        Meta Model API (Muse Spark) — suggest mode / auto-tune loop
```

Non-negotiables: deterministic fixed timestep (1 ms physics / 10 ms controller, configurable) · seeded RNG with TRNG-seed option · replay = compact frame+event log (MessagePack, JSON export) · ≥10× realtime headless · units = meters internally, cm in UI.

## 4. Eight-day plan (you are on day 2)

| Day | Backend (Claude + subagents) | Frontend (Kimi) | Humans (you + teammate) |
|---|---|---|---|
| D2 (today) | Workspace skeleton, types crate, **draft API.md**, maze gen core | Scaffold, router, mock server from draft contract | Read prompts, kick off both agents, get API keys |
| D3 | Maze gen 2027 features + property tests; robot config; sim core kinematics | Robot Builder + Maze Studio UIs against mocks | Convert YOUR real robot code via the conversion prompt — be the first user |
| D4 | Sensor models (ToF/IMU/wheels/color), error profiles, replay recorder | Sim Theater: 2D view, telemetry HUD, WS client | Feed real-world noise numbers from your robot logs into presets |
| D5 | Controller import (Rhai + validation), scoring engine, LoP/restarts | Replay scrubber, event log, sensor overlays | End-to-end smoke: generate → place → import → run → replay |
| D6 | Muse tuner: dossier compressor, suggest mode, schema'd responses | Compare Lab + Tuning Lab UIs | Write 2nd algorithm (e.g. wall-follower vs bug-style) for the A/B demo |
| D7 | Auto-tune loop + guardrails, batch CLI, perf pass | Ghost overlay, polish, demo mode, error states | Record the money demo: fail → Muse diagnoses → auto-fix → score up |
| D8 | Freeze, docs, contract tests green, seed a "demo pack" (robot+maze+runs) | Freeze, Lighthouse pass, screenshots | Submission video, README, poster-style GIFs |

## 5. Deliverables in this folder

| File | Goes to | Purpose |
|---|---|---|
| `SUMMARY.md` | you | this research + verdict |
| `PROMPT1.md` | **Kimi K3.1 Pro** | full frontend build prompt (UI/UX spec, draft contract, mock mode) |
| `PROMPT2.md` | **Claude Opus (backend lead)** | full backend build prompt (subagent plan, sim/sensor/maze specs, Muse integration, must author `docs/API.md`) |
| `CLAUDE.md` | repo root | standing instructions for Claude Code in the repo |
| `AGENTS.md` | repo root | tool-agnostic agent guide + front/back coordination protocol |
| `SKILLS.md` | repo root | reusable agent skill recipes (add a sensor, debug nondeterminism, run a bake-off…) |

> Note: you wrote "SKILS.md" — I used the correct spelling `SKILLS.md`.

## 6. Bottom line

**Idea quality: strong.** It targets a real pain (RCJ teams have no faithful at-home practice tool), it uses the hackathon's own models where they genuinely shine (structured failure analysis + tool-use tuning), and the 2027 rule changes (28 cm tiles, smiley victims, blue tiles, Dangerous Zone) give you a timely, specific hook no existing tool covers. **Viability: conditional.** With the TRNG→seeded-CSPRNG fix, the hard controller contract, kinematic-not-rigid-body physics, and the day-by-day cuts above, an 8-day MVP is realistic — especially with two AI agents on parallel tracks behind one frozen API contract. Without those fixes, it becomes a non-reproducible, non-integrating science fair. Build the boring contract first, the Muse magic second, and the demo will sell itself.
