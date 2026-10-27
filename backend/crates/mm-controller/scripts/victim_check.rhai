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
