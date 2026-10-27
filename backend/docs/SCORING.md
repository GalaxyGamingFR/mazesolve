# Scoring: rule -> code (RCJ Rescue Maze 2027 draft, §5.6)

All numbers come from `rules_profiles/rcj_maze_2027_draft.json` (`RulesProfile`). Code lives in
`crates/mm-sim/src/game.rs` (`GameState`), called from `sim.rs`. Events are time-stamped by the sim.

| Rule | Behaviour | Code |
|---|---|---|
| 5.6.1 identification | `cmds.report` is valid if over the last `play.identification_light_s` the robot was stationary (`speed < play.stationary_mps` every tick) and `id_light` matched the on/off blink pattern (`identification_blink_on_s` / `_off_s`) for >= 80 % of ticks, with +-1 tick jitter per edge and any phase. Otherwise a `Note`, nothing else. The window is cleared after every report. | `identification_valid`, `handle_report` |
| target | Nearest victim within `scoring.identify_within_m` of the robot centre, on the robot's tile or a neighbour (8-neighbourhood). | `handle_report` |
| 5.6.2 points | `victim_points` by tier (`Victim.tier`, else tile class) and kind (smiley / cognitive). Each victim scores once. | `victim_points` |
| truth status | Smiley: `Victim.status`. Cognitive: ring sum via `rules.ring_value` (2 harmed, 1 stable, 0 unharmed). | `truth_status` |
| 5.6.14 wrong status | `VictimIdentified{correct:false, points:0}` + `scoring.misidentification` (D4). | `handle_report` |
| no victim in range | `Misidentification` event + penalty. | `handle_report` |
| decoy in range | `DecoyFalsePositive` + penalty (once per decoy). | `handle_report` |
| 5.6.3-5 kits | `deploy_kits` goes to the most recently correctly identified victim within `play.kit_max_distance_m`. Points are cumulative `kit_points_by_count[min(total, kits_max[status]) - 1]`, so only the delta of counted kits scores. All requested kits (up to `kits_left`) are deducted. No victim near: `KitDeployed{victim_id:None, points:0}`. | `handle_kits` |
| 5.6.6 blue tile | After entering, the robot must be stationary for `field.blue_stop_s` (continuous). Leaving earlier: `BlueStop{ok:false}` and a LoP restart if LoP is on (D2). First ok visit `blue_tile`; each later ok visit subtracts `blue_revisit_penalty` from that tile's total, floor 0. | `update_blue`, `on_tile_exit` |
| ramp / stairs / bump | On leaving a feature tile after the centre moved >= 75 % of the tile along the feature axis from where it entered. Ramp: only upward (`up_dir`), event `up:true`. Once per tile. | `on_tile_enter`, `on_tile_exit` |
| checkpoint | Silver tile, first entry. | `on_tile_enter` |
| 5.6.12 exit | Robot on the start tile, `id_light` on every tick and stationary for `play.exit_light_s`: run ends `Exited` and the bonus `exit_per_svi*SVI + exit_per_sbv*SBV + exit_per_ssn*SSN + exit_per_srn*SRN` is added. `end_run` while not satisfied: `Note`, run continues. | `on_commands`, `finish`, `Sim::step` |
| 5.6.7 reliability | At finish: `max(0, SVI*a + SRD*b + SBV*c - LoP*d)`. SRD = kits that counted for points. SBV = distinct blue tiles with an ok visit. | `finish` |
| missed victims | At finish, `VictimMissed` for each real victim not correctly identified. | `finish` |
| 5.6.14 floor | `ScoreBreakdown::sum` clamps the total at 0. | mm-core |

`Metrics` fields `victims_identified`, `misidentifications` (wrong status, no victim, decoy),
`decoy_false_positives` and `kits_used` come from `GameState.stats` via `Sim::metrics`.

## Sensors feeding the controller
- Colour (`sensors/color.rs`): AS7343, 14 channels, counts = signature x gain x integration_ms x 100 x ambient,
  noise `sigma_frac`, per-run ambient jitter, optional flicker; reads a 6 mm spot under the mount. Ramps and stairs are white.
- Camera (`sensors/camera.rs`): range, hfov/vfov, raycast line of sight, facing test; recall (blur above `blur_omega`),
  class/ring/decoy confusion, ghost detections, `latency_ms` delay, distance/bearing noise unless the profile is ideal.
