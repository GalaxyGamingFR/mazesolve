//! Built-in Rust reference controllers. `WallFollower` mirrors `scripts/wall_follower.rhai`.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use mm_core::{Commands, Controller, Observations, RobotInfo, TofReading, TOF_OUT_OF_RANGE_MM};

/// Does nothing: zero speed, light off.
#[derive(Debug, Default, Clone, Copy)]
pub struct Idle;

impl Controller for Idle {
    fn init(&mut self, _info: &RobotInfo) -> Result<(), String> {
        Ok(())
    }

    fn step(&mut self, _obs: &Observations) -> Result<Commands, String> {
        Ok(Commands::default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    Decide,
    Turn,
    Forward,
}

/// Right-hand rule on tile-centred moves: ToF front/left/right (`tof0`, `tof2`, `tof6`),
/// IMU yaw for turns and heading hold, wheel encoders for the one-tile distance.
#[derive(Debug, Clone)]
pub struct WallFollower {
    phase: Phase,
    heading: f64,
    target: f64,
    start_odo: f64,
    tile_m: f64,
    m_per_tick: f64,
    speed: f64,
    turn_speed: f64,
    turn_gain: f64,
    open_dist: f64,
    heading_gain: f64,
}

impl Default for WallFollower {
    fn default() -> Self {
        Self {
            phase: Phase::Start,
            heading: 0.0,
            target: 0.0,
            start_odo: 0.0,
            tile_m: 0.28,
            m_per_tick: 1e-4,
            speed: 0.15,
            turn_speed: 0.20,
            turn_gain: 0.50,
            open_dist: 0.22,
            heading_gain: 0.50,
        }
    }
}

impl WallFollower {
    pub fn new() -> Self {
        Self::default()
    }
}

fn wrap(a: f64) -> f64 {
    let t = a.rem_euclid(TAU);
    if t > PI {
        t - TAU
    } else {
        t
    }
}

/// Nearest of the four centre zones in metres; 10 m when nothing is seen.
fn centre_m(t: Option<&TofReading>) -> f64 {
    let Some(t) = t else { return 10.0 };
    let n = (t.zones.len() as f64).sqrt().round() as usize;
    if n < 2 {
        return 10.0;
    }
    let h = n / 2;
    let mut best = 10.0_f64;
    for r in [h - 1, h] {
        for c in [h - 1, h] {
            if let Some(&mm) = t.zones.get(r * n + c) {
                if mm != TOF_OUT_OF_RANGE_MM {
                    best = best.min(f64::from(mm) / 1000.0);
                }
            }
        }
    }
    best
}

fn tof<'a>(obs: &'a Observations, id: &str, idx: usize) -> Option<&'a TofReading> {
    obs.tof
        .iter()
        .find(|t| t.sensor_id == id)
        .or_else(|| obs.tof.get(idx))
}

fn param(info: &RobotInfo, name: &str, default: f64) -> f64 {
    info.params
        .get(name)
        .or_else(|| info.params.get(&format!("controller.{name}")))
        .copied()
        .unwrap_or(default)
}

impl Controller for WallFollower {
    fn init(&mut self, info: &RobotInfo) -> Result<(), String> {
        *self = Self::default();
        self.tile_m = info.tile_m;
        let w = &info.robot.wheels[0];
        self.m_per_tick = TAU * w.radius_m / f64::from(w.encoder_ticks_per_rev.max(1));
        self.speed = param(info, "speed", self.speed);
        self.turn_speed = param(info, "turn_speed", self.turn_speed);
        self.turn_gain = param(info, "turn_gain", self.turn_gain);
        self.open_dist = param(info, "open_dist", self.open_dist);
        self.heading_gain = param(info, "heading_gain", self.heading_gain);
        Ok(())
    }

    fn step(&mut self, obs: &Observations) -> Result<Commands, String> {
        let yaw = obs.imu.yaw;
        let odo = obs.encoders.iter().sum::<i64>() as f64 / 4.0 * self.m_per_tick;
        if self.phase == Phase::Start || obs.lop_restarted {
            self.phase = Phase::Decide;
            self.heading = yaw;
        }
        let (mut left, mut right) = (0.0, 0.0);

        if self.phase == Phase::Decide {
            let f = centre_m(tof(obs, "tof0", 0));
            let l = centre_m(tof(obs, "tof2", 2));
            let r = centre_m(tof(obs, "tof6", 6));
            if r > self.open_dist {
                self.target = wrap(self.heading - FRAC_PI_2);
                self.phase = Phase::Turn;
            } else if f > self.open_dist {
                self.start_odo = odo;
                self.phase = Phase::Forward;
            } else if l > self.open_dist {
                self.target = wrap(self.heading + FRAC_PI_2);
                self.phase = Phase::Turn;
            } else {
                self.target = wrap(self.heading + PI);
                self.phase = Phase::Turn;
            }
        }

        if self.phase == Phase::Turn {
            let err = wrap(self.target - yaw);
            if err.abs() < 0.04 {
                self.heading = self.target;
                self.start_odo = odo;
                self.phase = Phase::Forward;
            } else {
                let mut w = (self.turn_gain * err).clamp(-self.turn_speed, self.turn_speed);
                if w.abs() < 0.04 {
                    w = if w < 0.0 { -0.04 } else { 0.04 };
                }
                left = -w;
                right = w;
            }
        }

        if self.phase == Phase::Forward {
            let front = centre_m(tof(obs, "tof0", 0));
            if odo - self.start_odo >= self.tile_m - 0.01 || front < 0.06 {
                self.phase = Phase::Decide;
            } else {
                let corr = (self.heading_gain * wrap(self.heading - yaw)).clamp(-0.1, 0.1);
                left = self.speed - corr;
                right = self.speed + corr;
            }
        }

        Ok(Commands {
            wheel_v: [left, right, left, right],
            ..Commands::default()
        })
    }
}
