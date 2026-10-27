//! Kinematic physics backend: per-wheel DDSM210 model plus skid-steer integration.
//! `PhysicsBackend` is the seam where a Rapier backend can drop in later.

use mm_core::{RobotConfig, TileKind, Vec2, WheelErrors, WheelState};

use crate::rng::{SimRng, Stream};
use crate::world::World;

/// Wheel speed noise is redrawn this often (sim seconds).
const NOISE_PERIOD_S: f64 = 0.05;
/// Minimum time between Collision events during sustained contact.
pub const COLLISION_DEBOUNCE_S: f64 = 0.25;
/// Floor slope (dz/ds) above which a wheel contact counts as a jolt (bump, stairs).
const JOLT_SLOPE: f64 = 0.8;

/// Ground-truth body state. Yaw is unwrapped (continuous).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BodyState {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub roll: f64,
    pub pitch: f64,
    pub yaw: f64,
    /// Forward speed actually achieved, m/s.
    pub v: f64,
    /// Yaw rate, rad/s.
    pub omega: f64,
}

/// What happened during one physics step.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StepInfo {
    /// Set when a (debounced) collision event should be emitted.
    pub collision: Option<Vec2>,
    /// A bump, stair edge or collision shook the chassis.
    pub jolt: bool,
    /// Distance moved this step, meters.
    pub moved: f64,
}

pub trait PhysicsBackend: Send {
    fn set_targets(&mut self, wheel_v: [f64; 4]);
    fn step(&mut self, world: &World, t: f64, dt: f64) -> StepInfo;
    fn body(&self) -> &BodyState;
    /// Move to `p` keeping yaw, stopping the wheels.
    fn teleport(&mut self, world: &World, p: Vec2);
    fn encoders(&self) -> [i64; 4];
    fn wheel_states(&self) -> [WheelState; 4];
}

#[derive(Debug, Clone, Default)]
struct Wheel {
    cmd: f64,
    actual: f64,
    noise: f64,
    next_noise_t: f64,
    ticks_f: f64,
    ticks: i64,
}

pub struct KinematicBackend {
    body: BodyState,
    err: WheelErrors,
    radius: f64,
    wheels: [Wheel; 4],
    pos: [(f64, f64); 4],
    wheel_r: [f64; 4],
    vmax: [f64; 4],
    tpr: [f64; 4],
    track: f64,
    base: f64,
    wheel_rng: Vec<Stream>,
    coll_rng: Stream,
    last_collision_t: f64,
    prev_z: [f64; 4],
}

impl KinematicBackend {
    pub fn new(
        robot: &RobotConfig,
        err: &WheelErrors,
        rng: &SimRng,
        world: &World,
        start: Vec2,
        yaw: f64,
    ) -> Self {
        let w = &robot.wheels;
        let track = ((w[0].y + w[2].y) - (w[1].y + w[3].y)) / 2.0;
        let base = ((w[0].x + w[1].x) - (w[2].x + w[3].x)) / 2.0;
        let mut b = KinematicBackend {
            body: BodyState {
                x: start.x,
                y: start.y,
                yaw,
                ..Default::default()
            },
            err: err.clone(),
            radius: robot.body.collision_radius_m,
            wheels: Default::default(),
            pos: [
                (w[0].x, w[0].y),
                (w[1].x, w[1].y),
                (w[2].x, w[2].y),
                (w[3].x, w[3].y),
            ],
            wheel_r: [w[0].radius_m, w[1].radius_m, w[2].radius_m, w[3].radius_m],
            vmax: [0.0; 4],
            tpr: [0.0; 4],
            track: if track > 1e-6 { track } else { 0.16 },
            base: if base > 1e-6 { base } else { 0.12 },
            wheel_rng: (0..4).map(|i| rng.stream(&format!("wheel/{i}"))).collect(),
            coll_rng: rng.stream("collision"),
            last_collision_t: f64::NEG_INFINITY,
            prev_z: [0.0; 4],
        };
        for (i, ws) in w.iter().enumerate() {
            b.vmax[i] = ws.max_rpm * std::f64::consts::TAU / 60.0 * ws.radius_m;
            b.tpr[i] = ws.encoder_ticks_per_rev as f64;
        }
        for wh in &mut b.wheels {
            wh.noise = 1.0;
        }
        b.settle_height(world, 0.0);
        b
    }

    fn contact_points(&self) -> [(f64, f64); 4] {
        let (s, c) = self.body.yaw.sin_cos();
        let mut out = [(0.0, 0.0); 4];
        for (o, &(wx, wy)) in out.iter_mut().zip(self.pos.iter()) {
            *o = (self.body.x + c * wx - s * wy, self.body.y + s * wx + c * wy);
        }
        out
    }

    /// z/roll/pitch from the heightfield under the four wheel contacts; returns jolt.
    fn settle_height(&mut self, world: &World, moved: f64) -> bool {
        let cp = self.contact_points();
        let mut z = [0.0; 4];
        for (zi, &(x, y)) in z.iter_mut().zip(cp.iter()) {
            *zi = world.floor_z(x, y).0;
        }
        self.body.z = z.iter().sum::<f64>() / 4.0;
        self.body.pitch = (((z[0] + z[1]) - (z[2] + z[3])) / 2.0).atan2(self.base);
        self.body.roll = (((z[0] + z[2]) - (z[1] + z[3])) / 2.0).atan2(self.track);
        let jolt = moved > 1e-5
            && z.iter()
                .zip(self.prev_z.iter())
                .any(|(a, b)| (a - b).abs() / moved > JOLT_SLOPE);
        self.prev_z = z;
        jolt
    }
}

impl PhysicsBackend for KinematicBackend {
    fn set_targets(&mut self, wheel_v: [f64; 4]) {
        for (w, v) in self.wheels.iter_mut().zip(wheel_v) {
            w.cmd = if v.is_finite() { v } else { 0.0 };
        }
    }

    fn step(&mut self, world: &World, t: f64, dt: f64) -> StepInfo {
        for i in 0..4 {
            let e = &self.err;
            let w = &mut self.wheels[i];
            if t >= w.next_noise_t {
                w.noise = 1.0 + e.speed_sigma_frac * self.wheel_rng[i].gauss();
                w.next_noise_t = t + NOISE_PERIOD_S;
            }
            let mut target = w.cmd.clamp(-self.vmax[i], self.vmax[i]);
            if target.abs() < e.deadband_mps {
                target = 0.0;
            }
            target *= w.noise;
            let lim = e.max_accel_mps2 * dt;
            w.actual += (target - w.actual).clamp(-lim, lim);
            w.ticks_f += w.actual * dt / (std::f64::consts::TAU * self.wheel_r[i]) * self.tpr[i];
            w.ticks = w.ticks_f.round() as i64;
        }
        let wa = |i: usize| self.wheels[i].actual;
        let left = (wa(0) + wa(2)) / 2.0;
        let right = (wa(1) + wa(3)) / 2.0;
        let slip = if world.on_ramp(self.body.x, self.body.y) {
            self.err.slip_ramp
        } else if world.surface_at(self.body.x, self.body.y) == TileKind::Blue {
            self.err.slip_blue
        } else {
            self.err.slip_normal
        };
        let v = (left + right) / 2.0 * (1.0 - slip);
        let omega = (right - left) / self.track * (1.0 - slip);

        let (x0, y0) = (self.body.x, self.body.y);
        let ymid = self.body.yaw + omega * dt / 2.0;
        self.body.x += v * ymid.cos() * dt;
        self.body.y += v * ymid.sin() * dt;
        self.body.yaw += omega * dt;

        let mut touch = None;
        for _ in 0..4 {
            let c = Vec2 {
                x: self.body.x,
                y: self.body.y,
            };
            match world.penetration(c, self.radius) {
                Some(p) if p.depth > 1e-9 => {
                    self.body.x += p.normal.x * p.depth;
                    self.body.y += p.normal.y * p.depth;
                    touch = Some(p.point);
                }
                _ => break,
            }
        }
        let mut collision = None;
        if let Some(p) = touch {
            if t - self.last_collision_t >= COLLISION_DEBOUNCE_S {
                self.last_collision_t = t;
                collision = Some(p);
                self.body.yaw += self.err.collision_yaw_kick * self.coll_rng.gauss();
            }
        }
        let moved = ((self.body.x - x0).powi(2) + (self.body.y - y0).powi(2)).sqrt();
        let (hs, hc) = ymid.sin_cos();
        self.body.v = ((self.body.x - x0) * hc + (self.body.y - y0) * hs) / dt;
        self.body.omega = omega;
        let bump = self.settle_height(world, moved);
        StepInfo {
            collision,
            jolt: bump || collision.is_some(),
            moved,
        }
    }

    fn body(&self) -> &BodyState {
        &self.body
    }

    fn teleport(&mut self, world: &World, p: Vec2) {
        self.body.x = p.x;
        self.body.y = p.y;
        self.body.v = 0.0;
        self.body.omega = 0.0;
        for w in &mut self.wheels {
            w.actual = 0.0;
        }
        self.settle_height(world, 0.0);
    }

    fn encoders(&self) -> [i64; 4] {
        [
            self.wheels[0].ticks,
            self.wheels[1].ticks,
            self.wheels[2].ticks,
            self.wheels[3].ticks,
        ]
    }

    fn wheel_states(&self) -> [WheelState; 4] {
        let f = |i: usize| WheelState {
            cmd: self.wheels[i].cmd,
            actual: self.wheels[i].actual,
        };
        [f(0), f(1), f(2), f(3)]
    }
}
