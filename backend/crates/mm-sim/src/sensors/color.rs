//! AS7343 colour sensor: 14-channel spectral reading of the floor under the mount.
//!
//! Each floor surface has a relative 14-channel signature (reflectance-like, 0..1) in AS7343
//! order: F1 415, F2 445, F3 480, F4 515, F5 555, F6 590, F7 630, F8 680 nm, FZ, FY, FXL,
//! NIR, Clear, Flicker. Counts = signature x gain x integration_ms x `COUNT_SCALE` x ambient,
//! with per-channel Gaussian noise, clamped to the 16-bit ADC range. Ramps and stairs are
//! white floor. The sensor sees a small disc under its mount: five points are averaged, so a
//! tile edge gives a blend.

use mm_core::{ColorErrors, ColorMount, ColorReading, RobotConfig, TileKind};

use crate::kinematics::BodyState;
use crate::rng::{SimRng, Stream};
use crate::world::World;

/// Counts per (gain x ms) for a unit-reflectance channel.
pub const COUNT_SCALE: f64 = 100.0;
const ADC_MAX: f64 = 65535.0;
/// Radius of the sensed disc under the mount, meters.
const SPOT_RADIUS_M: f64 = 0.006;
/// Mains flicker frequency and depth when `ColorErrors::flicker` is set.
const FLICKER_HZ: f64 = 100.0;
const FLICKER_DEPTH: f64 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    White,
    Black,
    Silver,
    Blue,
    Red,
}

impl Surface {
    pub fn of(kind: TileKind) -> Surface {
        match kind {
            TileKind::Normal | TileKind::Start => Surface::White,
            TileKind::Black => Surface::Black,
            TileKind::Silver => Surface::Silver,
            TileKind::Blue => Surface::Blue,
            TileKind::Red => Surface::Red,
        }
    }

    /// Relative signature, AS7343 channel order (Flicker channel is added separately).
    pub fn signature(self) -> [f64; 14] {
        match self {
            Surface::White => [
                0.80, 0.88, 0.90, 0.91, 0.92, 0.92, 0.91, 0.88, 0.88, 0.92, 0.92, 0.80, 0.90, 0.0,
            ],
            Surface::Silver => [
                0.50, 0.54, 0.56, 0.57, 0.57, 0.57, 0.56, 0.55, 0.54, 0.57, 0.57, 0.52, 0.55, 0.0,
            ],
            Surface::Black => [
                0.03, 0.03, 0.03, 0.03, 0.03, 0.03, 0.03, 0.03, 0.03, 0.03, 0.03, 0.04, 0.03, 0.0,
            ],
            Surface::Blue => [
                0.22, 0.45, 0.55, 0.35, 0.12, 0.06, 0.04, 0.05, 0.45, 0.12, 0.05, 0.08, 0.25, 0.0,
            ],
            Surface::Red => [
                0.03, 0.03, 0.04, 0.05, 0.06, 0.18, 0.55, 0.60, 0.03, 0.06, 0.30, 0.65, 0.28, 0.0,
            ],
        }
    }
}

struct ColorSensor {
    mount: ColorMount,
    err: ColorErrors,
    rng: Stream,
    /// Per-run ambient multiplier drawn once.
    ambient: f64,
    period_idx: u64,
    ch: [f32; 14],
    fresh: bool,
    frame_pending: bool,
}

impl ColorSensor {
    fn new(mount: &ColorMount, err: &ColorErrors, rng: &SimRng) -> Self {
        let mut s = rng.stream(&format!("color/{}", mount.id));
        let ambient = 1.0 + err.ambient_jitter * s.range(-1.0, 1.0);
        ColorSensor {
            mount: mount.clone(),
            err: err.clone(),
            rng: s,
            ambient,
            period_idx: 0,
            ch: [0.0; 14],
            fresh: false,
            frame_pending: false,
        }
    }

    fn due(&self, t: f64) -> bool {
        t >= self.period_idx as f64 / self.mount.rate_hz - 1e-9
    }

    fn sample(&mut self, t: f64, body: &BodyState, world: &World) {
        self.period_idx = (t * self.mount.rate_hz + 1e-9).floor() as u64 + 1;
        let (sy, cy) = body.yaw.sin_cos();
        let p = &self.mount.pose;
        let cx = body.x + cy * p.x - sy * p.y;
        let cyy = body.y + sy * p.x + cy * p.y;
        let mut sig = [0.0f64; 14];
        let offsets = [
            (0.0, 0.0),
            (SPOT_RADIUS_M, 0.0),
            (-SPOT_RADIUS_M, 0.0),
            (0.0, SPOT_RADIUS_M),
            (0.0, -SPOT_RADIUS_M),
        ];
        for (dx, dy) in offsets {
            let s = Surface::of(world.surface_at(cx + dx, cyy + dy)).signature();
            for (a, b) in sig.iter_mut().zip(s) {
                *a += b / offsets.len() as f64;
            }
        }
        let flick = (std::f64::consts::TAU * FLICKER_HZ * t).sin();
        let gain_mul = if self.err.flicker {
            1.0 + FLICKER_DEPTH * flick
        } else {
            1.0
        };
        let scale = self.mount.gain * self.mount.integration_ms * COUNT_SCALE * self.ambient;
        // Fixed draw count per sample so the stream stays aligned.
        let noise: Vec<f64> = (0..14).map(|_| self.rng.gauss()).collect();
        for (i, out) in self.ch.iter_mut().enumerate() {
            let mut v = sig[i] * scale * gain_mul * (1.0 + self.err.sigma_frac * noise[i]);
            if i == 13 {
                // Flicker channel: modulation depth in counts when flicker is present.
                v = if self.err.flicker {
                    sig[12] * scale * FLICKER_DEPTH * flick.abs()
                } else {
                    0.0
                };
            }
            *out = v.clamp(0.0, ADC_MAX).round() as f32;
        }
        self.fresh = true;
        self.frame_pending = true;
    }

    fn reading(&self, fresh: bool) -> ColorReading {
        ColorReading {
            sensor_id: self.mount.id.clone(),
            ch: self.ch,
            fresh,
        }
    }
}

/// All colour sensors of the robot.
pub struct ColorArray {
    sensors: Vec<ColorSensor>,
}

impl ColorArray {
    pub fn new(robot: &RobotConfig, err: &ColorErrors, rng: &SimRng) -> Self {
        ColorArray {
            sensors: robot
                .color
                .iter()
                .map(|m| ColorSensor::new(m, err, rng))
                .collect(),
        }
    }

    /// Take a sample on every sensor whose period has elapsed at sim time `t`.
    pub fn sample_due(&mut self, t: f64, body: &BodyState, world: &World) {
        for s in &mut self.sensors {
            if s.due(t) {
                s.sample(t, body, world);
            }
        }
    }

    /// Latest sample per sensor; `fresh` is true once per new sample.
    pub fn observe(&mut self) -> Vec<ColorReading> {
        self.sensors
            .iter_mut()
            .map(|s| {
                let r = s.reading(s.fresh);
                s.fresh = false;
                r
            })
            .collect()
    }

    /// Samples taken since the previous frame, consumed.
    pub fn drain_frame(&mut self) -> Vec<ColorReading> {
        self.sensors
            .iter_mut()
            .filter(|s| s.frame_pending)
            .map(|s| {
                s.frame_pending = false;
                s.reading(true)
            })
            .collect()
    }
}
