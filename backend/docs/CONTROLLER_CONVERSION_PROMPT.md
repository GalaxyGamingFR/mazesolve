# MazeMuse controller conversion prompt

Copy everything below the line into any AI agent (ChatGPT, Claude, Gemini, Muse, a local model), then paste your robot firmware (Arduino C++ or MicroPython) after it. The agent answers with one Rhai script that you paste into the MazeMuse editor.

---

You convert robot firmware into a **MazeMuse controller script**. MazeMuse simulates a RoboCup Junior Rescue Maze robot. The controller is a script in **Rhai** (a small Rust-embedded scripting language that looks like Rust/JavaScript). The firmware I give you after this prompt is Arduino C++ or MicroPython. Rewrite its behaviour as one Rhai script that follows the contract below exactly. Reply with **only the script, in one code block**, nothing before or after it.

## 1. The contract

A script defines two functions. The simulator calls `step` once per controller tick (10 ms of simulated time, `info.dt`). It never blocks: one call to `step` is one pass through what used to be your `loop()`.

```rhai
// @tunable wall_dist 0.10 0.05 0.20 0.01 "Target side-wall distance (m)"
fn init(info) {            // optional, called once. info: #{ tile_m, dt, params, robot }
    this.state = "explore";
}
fn step(obs) {             // required, called every tick. obs: see section 2
    let d = obs.tof[0].zones[27];     // millimetres
    #{ wheel_v: [0.2, 0.2, 0.2, 0.2], id_light: false }   // missing keys = defaults
}
```

- **State**: the only place state survives between ticks is the `this` object map (`this.state`, `this.t0`, ...). Functions cannot see global variables; Rhai has no globals. Everything a function needs is passed as an argument or read from `this`/`PARAMS`.
- **Return value**: an object map `#{ ... }` with any of the keys in section 3, or `()` for "all defaults" (stopped, light off). Missing keys take their default. Unknown keys are an error that names them. Non-finite wheel speeds (NaN, infinity) are an error.
- **Tunables**: constants worth tuning are declared in comment lines (section 5) and read as `PARAMS.name`.
- **Errors end the run** (controller fault) and are shown with line and column. Run budget: about 200,000 operations per step, so no unbounded `loop {}` / `while true` inside a step.
- **No I/O**: no files, no network, no clock, no random numbers. `print(x)` and `debug(x)` are captured into a log (at most 64 short lines per step); use sparingly.

## 2. `info` and `obs`

`info` (given to `init`): `info.tile_m` (float, tile side in m, 0.28), `info.dt` (float, 0.01 s), `info.params` (map of tunable name to float), `info.robot` (the robot config: `info.robot.wheels[i].radius_m`, `.encoder_ticks_per_rev`, `info.robot.kits`, `info.robot.tof`, ...).

`PARAMS` is a global constant holding the same resolved tunables as `info.params`; it works inside every function.

`obs` (given to `step`), all SI units:

| Field | Type / unit | Meaning |
|---|---|---|
| `obs.t` | float, s | Sim time since the run started. Use it for every timer. |
| `obs.tof` | array of maps | One per ToF sensor (reference robot: 8). |
| `obs.tof[i].sensor_id` | string | `"tof0"` ... `"tof7"` |
| `obs.tof[i].zones` | array of int, **mm** | Row-major, 8x8 = 64 values (index `row*8 + col`), **row 0 = top**, col 0 = left as seen from the sensor. `65535` = no target / out of range. |
| `obs.tof[i].status` | array of int | Per-zone VL53L8CX status: `5` valid, `6` sigma fail, `7` wrap, `255` no target. |
| `obs.tof[i].fresh` | bool | `true` on the tick a new sample arrived (sensors run slower than 100 Hz). |
| `obs.color` | array of maps | Colour sensors (reference robot: 1, `obs.color[0]`). |
| `obs.color[i].ch` | array of 14 floats | Raw AS7343 counts, order: `F1 F2 F3 F4 F5 F6 F7 F8 FZ FY FXL NIR Clear Flicker` (indexes 0..13: F1=0 ... F8=7, FZ=8, FY=9, FXL=10, NIR=11, Clear=12, Flicker=13). |
| `obs.color[i].fresh` | bool | New sample this tick. |
| `obs.cams` | array of maps | Camera detections that arrived this tick (often empty). |
| `obs.cams[i].camera_id` | string | Which camera. |
| `obs.cams[i].kind` | string | `"smiley"` or `"cognitive"` |
| `obs.cams[i].status` | string or `()` | `"harmed"`, `"stable"`, `"unharmed"` for smileys; `()` for cognitive targets. |
| `obs.cams[i].rings` | array of 5 strings or `()` | Cognitive targets only: ring colours innermost first (`"black"`, `"red"`, `"yellow"`, `"green"`, `"blue"`). |
| `obs.cams[i].confidence` | float 0..1 | Detector confidence. |
| `obs.cams[i].distance_m` | float, m | Distance to the target. |
| `obs.cams[i].bearing` | float, rad | Angle in the camera frame, left positive. |
| `obs.cams[i].captured_t` | float, s | When the frame was taken (detections arrive late). |
| `obs.imu.roll`, `.pitch`, `.yaw` | float, **rad** | Fused attitude. `yaw` is counter-clockwise positive, 0 at the start heading, it may wrap at plus or minus pi, so always wrap angle differences into (-pi, pi]. |
| `obs.imu.gyro` | array of 3 floats, rad/s | Body rates x, y, z. |
| `obs.imu.accel` | array of 3 floats, m/s^2 | Specific force x, y, z. |
| `obs.encoders` | array of 4 ints, ticks | Cumulative per wheel, order **FL, FR, RL, RR**. |
| `obs.kits_left` | int | Rescue kits still on board. |
| `obs.lop_restarted` | bool | `true` for exactly one tick after a Lack-of-Progress restart: reset your navigation state. |

Reference robot geometry: **8 ToF sensors `tof0`..`tof7` at 45 degree steps counter-clockwise from forward** (`tof0` front, `tof1` front-left, `tof2` left, `tof3` rear-left, `tof4` rear, `tof5` rear-right, `tof6` right, `tof7` front-right). Four wheels in the order **FL, FR, RL, RR**. Look sensors up by `sensor_id` (see the examples) so custom robots still work. The centre of an 8x8 grid is zones 27, 28, 35, 36.

## 3. Return map (`cmds`)

| Key | Type / unit | Default | Meaning |
|---|---|---|---|
| `wheel_v` | array of 4 numbers, **m/s** | `[0,0,0,0]` | Wheel surface speed, order **FL, FR, RL, RR**. Positive = forward. Turning left (CCW) = left wheels slower than right wheels. |
| `left`, `right` | number, m/s | none | Shorthand: `left` drives FL and RL, `right` drives FR and RR. Do not combine with `wheel_v`. |
| `id_light` | bool | `false` | The identification light (blink it to identify victims). |
| `deploy_kits` | int >= 0 | `0` | Kits to drop this tick. |
| `report` | `"harmed"` / `"stable"` / `"unharmed"` / `()` | `()` | Claim an identified victim; the simulator scores it. |
| `end_run` | bool | `false` | Tell the simulator the robot believes it is finished on the start tile. |
| `note` | string | none | Free text saved in the replay for debugging. |

Numbers may be ints or floats (`0` or `0.0`). Typical top speed is under 0.4 m/s.

## 4. Mapping firmware to the script

| Firmware | Script |
|---|---|
| `setup()` | `fn init(info) { ... }` initialising `this.*` |
| `loop()` body | `fn step(obs) { ... }`, run once per 10 ms tick |
| global variables / static locals | fields of `this` (`this.count`, `this.state`) |
| `#define` / `const` thresholds and gains | `// @tunable` lines read as `PARAMS.name` (or plain literals if not worth tuning) |
| `vl53l8cx.get_ranging_data()` / `sensor.get_distance()` | `obs.tof[i].zones[...]` (mm); pick the sensor by `sensor_id` |
| `bno055.getVector(...)` / IMX-5 heading | `obs.imu.yaw` (rad, CCW positive). Convert degrees: `deg * 0.017453292519943295`. |
| `readColor()` / AS7343 channels | `obs.color[0].ch[...]` |
| camera / OpenMV result | `obs.cams[...]` |
| `analogWrite` / `motor.speed()` / servo drive | the `wheel_v` entry (m/s, not PWM: scale `pwm / 255 * max_speed`) |
| LED / `digitalWrite(LED, ...)` for the identification light | `id_light: true/false` |
| servo or solenoid that drops a rescue kit | `deploy_kits: n` |
| `delay(ms)` / `sleep_ms` | **Never block.** Store `this.t0 = obs.t` and return stopped or moving commands until `obs.t - this.t0 >= seconds`. Turn it into a state in `this.state`. |
| `millis()` / `ticks_ms()` | `obs.t * 1000.0` (or just work in seconds) |
| `while (cond) {...}` waiting for a sensor | one `if` per tick; return and let `step` be called again |
| interrupts / encoder ISRs | remove; read `obs.encoders` (cumulative ticks) and take differences |
| `Serial.print` / `print()` | `print("...")` sparingly, or `note: "..."` in the return map |
| I2C / SPI / UART setup, `Wire.begin()`, `pinMode`, `import machine` | delete |
| `random()` | not available; replace with a deterministic rule |
| `delay` between servo moves, PID loops with `micros()` | PID per tick using `info.dt` (0.01) as the time step |

A hardware feature with no counterpart (display, buzzer, SD card, Bluetooth) is dropped silently. If something important cannot be converted, write a comment line `// NOTE: ...` inside the script saying what was left out.

## 5. Tunables

Put thresholds, gains and speeds the user may want to tune in comment lines, **one per line, anywhere in the script**:

```
// @tunable name default min max max_step "description"
```

- `name`: letters, digits, `_`, not starting with a digit. Read it as `PARAMS.name`. The tuner sees it as `controller.name`.
- `default`, `min`, `max`, `max_step`: plain decimal numbers (no units, no expressions) with `min <= default <= max`. `max_step` is the largest change the tuner may make in one iteration (positive).
- `description` in double quotes, include the unit.
- Always declare at least the speeds, distance thresholds and PID gains. Do not declare things that must not change (the 5 s, 500 ms and 15 cm rule constants below).

Example: `// @tunable wall_dist 0.10 0.05 0.20 0.01 "Target side-wall distance (m)"`.

## 6. Rules the code must respect

1. **Blue tile**: when the robot enters a blue tile (detect it from the colour channels, e.g. blue channels F3+F4 much larger than F6+F8 and Clear above a floor) it must **stop for 5 seconds**, then continue.
2. **Identify a victim** by blinking the identification light **500 ms on, 500 ms off for 5 seconds** while the robot is **completely still** and the target is **within 15 cm** (`distance_m < 0.15`). Set `id_light` true/false from `obs.t`. Do not move during the blink. Then report with `report: "harmed"|"stable"|"unharmed"` (the status from the detection).
3. **Rescue kits**: after identifying, deploy **2 kits for a harmed victim, 1 for a stable victim, 0 for unharmed** (`deploy_kits`), never more than `obs.kits_left`. Do not re-identify the same victim: after finishing, ignore detections until the robot has moved about one tile.
4. **Exit bonus**: at the end, back on the **start tile**, the robot turns `id_light` on, stays **completely still for 5 seconds**, and may set `end_run: true`. Only do this if the firmware has an "I am finished" behaviour (for example after a timer or after all tiles are explored).
5. **Lack of Progress (LoP)**: a restart puts the robot back on the last checkpoint. When `obs.lop_restarted` is true, reset navigation state (current heading target, move in progress, timers) and plan again from the new position.
6. The robot is tile-based: tiles are `info.tile_m` (0.28 m) wide; moving "one tile" means about 0.28 m of travel.

## 7. Rhai syntax gotchas (for C/C++ and Python people)

- Variables: `let x = 1;` (no types). Assign without `let`. **Semicolons** end statements. Constants: `const K = 3;` only inside functions.
- **No `++` / `--`**. Write `i += 1;`. There is no ternary `?:`: use `if c { a } else { b }`, which is an expression. Endless loops are `loop { ... break; }`; avoid them inside `step`.
- **Functions do not see outer variables.** There are no globals. A helper `fn f(x) { x + K }` cannot read `K`. Pass what it needs as arguments, or use `PARAMS.k`. Inside `init`/`step`, `this` is available; **inside helper functions called normally, it is not**. Pass values in.
- **Integer vs float**: `5` is an int, `5.0` is a float. `7 / 2` is `3` (int division). `0.5 * 2` works (mixed arithmetic), but `to_float()` / `to_int()` are the conversions: `obs.tof[0].zones[27].to_float() / 1000.0`. Comparisons mix freely. `%` on floats works.
- Math: `sqrt(x)`, `sin`, `cos`, `atan2(y, x)`, `abs()` as a method (`x.abs()`), `min(a, b)`, `max(a, b)`, `floor`, `ceiling`, `round`. There is no `clamp`; write one (`if x < lo { lo } else if x > hi { hi } else { x }`). The constant pi is not predefined: write `3.141592653589793`.
- Arrays: `let a = [1, 2, 3]; a.len(); a.push(4); a[0]`. Indexing starts at 0. Maps: `#{ a: 1, b: 2 }`, access `m.a`; missing property is `()`, not an error. `()` is the "nothing" value; compare with `x == ()`.
- Strings use double quotes, compare with `==`, concatenate with `+`.
- `if`/`while`/`for` conditions take **no parentheses needed**, but braces are **mandatory**: `if x > 0 { ... }`. `for i in 0..8 { }` excludes 8; `0..=8` includes it. `for item in array { }` iterates values.
- Booleans: `&&`, `||`, `!`. A function's last expression (no semicolon) is its return value; or write `return x;`.
- Comments: `//` and `/* */`.
- A bad `step` return map (typo in a key) is an error, so use exactly the keys from section 3.

## 8. Worked example A: right-hand wall follower

C++ source it replaces (abridged): `if (right > 220) turnRight(); else if (front > 220) forward(); else if (left > 220) turnLeft(); else turnAround();` with `delay()`-timed turns and `analogWrite` motors. The script below does the same with tile-centred moves, yaw-controlled turns and a state machine in `this`.

```rhai
// Right-hand-rule wall follower on tile-centred moves.
// One step = one 10 ms controller tick. State lives in `this`.
// @tunable speed 0.15 0.05 0.30 0.01 "Forward wheel speed (m/s)"
// @tunable turn_speed 0.20 0.05 0.40 0.01 "Max wheel speed while turning in place (m/s)"
// @tunable turn_gain 0.50 0.10 2.00 0.05 "Turn speed per radian of heading error (m/s per rad)"
// @tunable open_dist 0.22 0.15 0.30 0.01 "Range above which a side or front is open (m)"
// @tunable heading_gain 0.50 0.00 2.00 0.05 "Heading hold gain while driving straight (m/s per rad)"

fn wrap_angle(a) {
    while a > 3.141592653589793 { a -= 6.283185307179586; }
    while a < -3.141592653589793 { a += 6.283185307179586; }
    a
}

fn clampf(x, lo, hi) {
    if x < lo { lo } else if x > hi { hi } else { x }
}

// Sensor lookup by id, falling back to its position in the list.
fn tof_by_id(obs, id, idx) {
    for t in obs.tof {
        if t.sensor_id == id { return t; }
    }
    if idx < obs.tof.len() { obs.tof[idx] } else { () }
}

// Nearest of the four centre zones in metres; 10.0 when nothing is seen.
fn range_m(t) {
    if t == () { return 10.0; }
    let n = sqrt(t.zones.len().to_float()).to_int();
    let h = n / 2;
    let best = 10.0;
    for r in [h - 1, h] {
        for c in [h - 1, h] {
            let mm = t.zones[r * n + c];
            if mm < 65535 {
                let m = mm.to_float() / 1000.0;
                if m < best { best = m; }
            }
        }
    }
    best
}

fn odo_m(enc, mpt) {
    (enc[0] + enc[1] + enc[2] + enc[3]).to_float() / 4.0 * mpt
}

fn init(info) {
    this.state = "start";
    this.heading = 0.0;
    this.target = 0.0;
    this.start_odo = 0.0;
    this.tile = info.tile_m;
    let w = info.robot.wheels[0];
    let ticks = w.encoder_ticks_per_rev.to_float();
    if ticks < 1.0 { ticks = 1.0; }
    this.mpt = 6.283185307179586 * w.radius_m / ticks;
}

fn step(obs) {
    let yaw = obs.imu.yaw;
    let odo = odo_m(obs.encoders, this.mpt);
    if this.state == "start" || obs.lop_restarted {
        this.state = "decide";
        this.heading = yaw;
    }
    let left = 0.0;
    let right = 0.0;

    if this.state == "decide" {
        let open = PARAMS.open_dist;
        let f = range_m(tof_by_id(obs, "tof0", 0));
        let l = range_m(tof_by_id(obs, "tof2", 2));
        let r = range_m(tof_by_id(obs, "tof6", 6));
        if r > open {
            this.target = wrap_angle(this.heading - 1.5707963267948966);
            this.state = "turn";
        } else if f > open {
            this.start_odo = odo;
            this.state = "forward";
        } else if l > open {
            this.target = wrap_angle(this.heading + 1.5707963267948966);
            this.state = "turn";
        } else {
            this.target = wrap_angle(this.heading + 3.141592653589793);
            this.state = "turn";
        }
    }

    if this.state == "turn" {
        let err = wrap_angle(this.target - yaw);
        if err.abs() < 0.04 {
            this.heading = this.target;
            this.start_odo = odo;
            this.state = "forward";
        } else {
            let w = clampf(PARAMS.turn_gain * err, -PARAMS.turn_speed, PARAMS.turn_speed);
            if w.abs() < 0.04 { w = if w < 0.0 { -0.04 } else { 0.04 }; }
            left = -w;
            right = w;
        }
    }

    if this.state == "forward" {
        let front = range_m(tof_by_id(obs, "tof0", 0));
        if odo - this.start_odo >= this.tile - 0.01 || front < 0.06 {
            this.state = "decide";
        } else {
            let corr = clampf(PARAMS.heading_gain * wrap_angle(this.heading - yaw), -0.1, 0.1);
            left = PARAMS.speed - corr;
            right = PARAMS.speed + corr;
        }
    }

    #{ wheel_v: [left, right, left, right] }
}
```

## 9. Worked example B: wall follower plus victims and blue tiles

Same navigation, plus the rules from section 6: stop 5 s on blue, and when a smiley is closer than 15 cm, stop, blink the light 500/500 ms for 5 s, report the status, deploy kits (2 harmed, 1 stable), then continue.

```rhai
// Wall follower plus victim identification and the blue-tile stop.
// One step = one 10 ms controller tick. State lives in `this`.
// @tunable speed 0.15 0.05 0.30 0.01 "Forward wheel speed (m/s)"
// @tunable turn_speed 0.20 0.05 0.40 0.01 "Max wheel speed while turning in place (m/s)"
// @tunable turn_gain 0.50 0.10 2.00 0.05 "Turn speed per radian of heading error (m/s per rad)"
// @tunable open_dist 0.22 0.15 0.30 0.01 "Range above which a side or front is open (m)"
// @tunable victim_dist 0.12 0.05 0.15 0.01 "Identify a victim closer than this (m); rules say within 15 cm"
// @tunable blue_ratio 1.30 1.00 3.00 0.05 "Blue tile when (F3+F4) exceeds this times (F6+F8)"
// @tunable blue_stop_s 5.0 5.0 8.0 0.5 "Stop time on a blue tile (s); rules require 5 s"
// @tunable heading_gain 0.50 0.00 2.00 0.05 "Heading hold gain while driving straight (m/s per rad)"

fn wrap_angle(a) {
    while a > 3.141592653589793 { a -= 6.283185307179586; }
    while a < -3.141592653589793 { a += 6.283185307179586; }
    a
}

fn clampf(x, lo, hi) {
    if x < lo { lo } else if x > hi { hi } else { x }
}

// Sensor lookup by id, falling back to its position in the list.
fn tof_by_id(obs, id, idx) {
    for t in obs.tof {
        if t.sensor_id == id { return t; }
    }
    if idx < obs.tof.len() { obs.tof[idx] } else { () }
}

// Nearest of the four centre zones in metres; 10.0 when nothing is seen.
fn range_m(t) {
    if t == () { return 10.0; }
    let n = sqrt(t.zones.len().to_float()).to_int();
    let h = n / 2;
    let best = 10.0;
    for r in [h - 1, h] {
        for c in [h - 1, h] {
            let mm = t.zones[r * n + c];
            if mm < 65535 {
                let m = mm.to_float() / 1000.0;
                if m < best { best = m; }
            }
        }
    }
    best
}

fn stopped(light) {
    #{ wheel_v: [0.0, 0.0, 0.0, 0.0], id_light: light }
}

// AS7343 channels: F1..F8 = 0..7, FZ 8, FY 9, FXL 10, NIR 11, Clear 12, Flicker 13.
fn is_blue(obs, ratio) {
    if obs.color.len() == 0 { return false; }
    let ch = obs.color[0].ch;
    ch[12] > 50.0 && ch[2] + ch[3] > ratio * (ch[5] + ch[7] + 1.0)
}

fn odo_m(enc, mpt) {
    (enc[0] + enc[1] + enc[2] + enc[3]).to_float() / 4.0 * mpt
}

fn init(info) {
    this.state = "start";
    this.heading = 0.0;
    this.target = 0.0;
    this.start_odo = 0.0;
    this.tile = info.tile_m;
    let w = info.robot.wheels[0];
    let ticks = w.encoder_ticks_per_rev.to_float();
    if ticks < 1.0 { ticks = 1.0; }
    this.mode = "drive";
    this.on_blue = false;
    this.blue_until = 0.0;
    this.victim_odo = -1000.0;
    this.ident_t0 = 0.0;
    this.ident_status = ();
    this.ident_kits = 0;
    this.mpt = 6.283185307179586 * w.radius_m / ticks;
}

fn step(obs) {
    let yaw = obs.imu.yaw;
    let odo = odo_m(obs.encoders, this.mpt);
    if this.state == "start" || obs.lop_restarted {
        this.state = "decide";
        this.heading = yaw;
    }
    let t = obs.t;

    // Blue tile: stop for blue_stop_s from the moment it is entered.
    let blue = is_blue(obs, PARAMS.blue_ratio);
    if blue && !this.on_blue {
        this.on_blue = true;
        this.blue_until = t + PARAMS.blue_stop_s;
    }
    if !blue { this.on_blue = false; }

    // Identification in progress: settle 0.2 s, blink 500 ms on / 500 ms off for 5 s, then report.
    if this.mode == "ident" {
        let el = t - this.ident_t0 - 0.2;
        if el < 0.0 { return stopped(false); }
        if el < 5.0 { return stopped((el / 0.5).to_int() % 2 == 0); }
        this.mode = "drive";
        this.victim_odo = odo;
        return #{
            wheel_v: [0.0, 0.0, 0.0, 0.0],
            id_light: false,
            report: this.ident_status,
            deploy_kits: this.ident_kits,
            note: "victim reported"
        };
    }

    if t < this.blue_until { return stopped(false); }

    // Start identifying a smiley that is close, unless we just handled one here.
    if odo - this.victim_odo > this.tile * 0.9 {
        for c in obs.cams {
            if c.kind == "smiley" && c.status != () && c.distance_m < PARAMS.victim_dist {
                let kits = if c.status == "harmed" { 2 } else if c.status == "stable" { 1 } else { 0 };
                if kits > obs.kits_left { kits = obs.kits_left; }
                this.mode = "ident";
                this.ident_t0 = t;
                this.ident_status = c.status;
                this.ident_kits = kits;
                return stopped(false);
            }
        }
    }

    let left = 0.0;
    let right = 0.0;

    if this.state == "decide" {
        let open = PARAMS.open_dist;
        let f = range_m(tof_by_id(obs, "tof0", 0));
        let l = range_m(tof_by_id(obs, "tof2", 2));
        let r = range_m(tof_by_id(obs, "tof6", 6));
        if r > open {
            this.target = wrap_angle(this.heading - 1.5707963267948966);
            this.state = "turn";
        } else if f > open {
            this.start_odo = odo;
            this.state = "forward";
        } else if l > open {
            this.target = wrap_angle(this.heading + 1.5707963267948966);
            this.state = "turn";
        } else {
            this.target = wrap_angle(this.heading + 3.141592653589793);
            this.state = "turn";
        }
    }

    if this.state == "turn" {
        let err = wrap_angle(this.target - yaw);
        if err.abs() < 0.04 {
            this.heading = this.target;
            this.start_odo = odo;
            this.state = "forward";
        } else {
            let w = clampf(PARAMS.turn_gain * err, -PARAMS.turn_speed, PARAMS.turn_speed);
            if w.abs() < 0.04 { w = if w < 0.0 { -0.04 } else { 0.04 }; }
            left = -w;
            right = w;
        }
    }

    if this.state == "forward" {
        let front = range_m(tof_by_id(obs, "tof0", 0));
        if odo - this.start_odo >= this.tile - 0.01 || front < 0.06 {
            this.state = "decide";
        } else {
            let corr = clampf(PARAMS.heading_gain * wrap_angle(this.heading - yaw), -0.1, 0.1);
            left = PARAMS.speed - corr;
            right = PARAMS.speed + corr;
        }
    }

    #{ wheel_v: [left, right, left, right] }
}
```

## 10. Output checklist (verify before you answer)

- [ ] Exactly one code block with the script, and **no other text** in the reply.
- [ ] `fn step(obs)` exists and takes exactly one argument; `fn init(info)` if there is setup.
- [ ] No blocking: no `delay`-style busy loops, no `loop {}` without a `break` that is reached within the same step; every wait is a state plus `obs.t`.
- [ ] All persistent variables live in `this.*` and are initialised in `init` (or guarded: `if this.state == () { ... }`).
- [ ] No helper function reads an outer variable or `this`; values are passed as arguments.
- [ ] Integer/float mixing is safe: sensor `zones` are ints in mm, converted with `.to_float() / 1000.0` for metres.
- [ ] Wheel order is FL, FR, RL, RR; speeds are m/s and finite; turning left means right wheels faster.
- [ ] Return map uses only these keys: `wheel_v`, `left`, `right`, `id_light`, `deploy_kits`, `report`, `end_run`, `note`.
- [ ] `// @tunable` lines are valid: six fields, quotes around the description, `min <= default <= max`, positive `max_step`.
- [ ] Rules in section 6 are kept (5 s blue stop, 500/500 ms blink for 5 s within 15 cm and still, kits 2/1/0, exit bonus, LoP reset).
- [ ] No `++`, no ternary, no `#include`, no `import`, no unknown functions (`millis`, `random`, `delay`, `Serial`).

Now convert the firmware below. Answer with only the script in one code block.

<!-- paste your Arduino C++ or MicroPython firmware here -->
