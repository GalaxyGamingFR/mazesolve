//! VL53L8CX time-of-flight model.
//!
//! Each sensor casts `resolution^2` rays spread evenly over a square field of view.
//! `fov_deg` is the datasheet diagonal (~45 deg); we use `fov_deg` per axis, documented
//! deviation that keeps zone geometry simple. Zones are row-major with row 0 at the top
//! (elevated) and column 0 at the left (positive azimuth). Range is the ray distance.
//! The floor is the plane z = 0, so ramps do not change what the floor looks like.

use mm_core::{
    tof_status, RobotConfig, TofErrors, TofMount, TofReading, Vec3, TOF_OUT_OF_RANGE_MM,
};

use crate::kinematics::BodyState;
use crate::rng::{SimRng, Stream};
use crate::world::{Rot3, World};

struct TofSensor {
    mount: TofMount,
    err: TofErrors,
    rng: Stream,
    /// Unit ray directions in the robot frame, row-major.
    dirs: Vec<[f64; 3]>,
    period_idx: u64,
    zones: Vec<u16>,
    status: Vec<u8>,
    fresh: bool,
    frame_pending: bool,
}

impl TofSensor {
    fn new(mount: &TofMount, err: &TofErrors, rng: &SimRng) -> Self {
        let n = mount.resolution as usize;
        let fov = mount.fov_deg.to_radians();
        let mrot = Rot3::from_ypr(mount.pose.yaw, mount.pose.pitch, 0.0);
        let mut dirs = Vec::with_capacity(n * n);
        for r in 0..n {
            let el = (0.5 - (r as f64 + 0.5) / n as f64) * fov;
            for c in 0..n {
                let az = (0.5 - (c as f64 + 0.5) / n as f64) * fov;
                let d = [el.cos() * az.cos(), el.cos() * az.sin(), el.sin()];
                dirs.push(mrot.apply(d));
            }
        }
        TofSensor {
            mount: mount.clone(),
            err: err.clone(),
            rng: rng.stream(&format!("tof/{}", mount.id)),
            dirs,
            period_idx: 0,
            zones: vec![TOF_OUT_OF_RANGE_MM; n * n],
            status: vec![tof_status::NO_TARGET; n * n],
            fresh: false,
            frame_pending: false,
        }
    }

    fn due(&self, t: f64) -> bool {
        t >= self.period_idx as f64 / self.mount.rate_hz - 1e-9
    }

    fn sample(&mut self, t: f64, body: &BodyState, world: &World) {
        self.period_idx = (t * self.mount.rate_hz + 1e-9).floor() as u64 + 1;
        let rot = Rot3::from_ypr(body.yaw, body.pitch, body.roll);
        let p = &self.mount.pose;
        let lo = rot.apply([p.x, p.y, p.z]);
        let origin = Vec3 {
            x: body.x + lo[0],
            y: body.y + lo[1],
            z: body.z + lo[2],
        };
        let max_m = self.mount.max_range_m;
        let max_mm = max_m * 1000.0;
        let count = self.dirs.len();
        // Fixed number of draws per sample whatever the outcome, so streams stay aligned.
        let blind = self.rng.chance(self.err.blind);
        let ghost = self.rng.chance(self.err.crosstalk);
        let ghost_zone = self.rng.below(count);
        let ghost_mm = self.rng.range(20.0, 80.0);
        for i in 0..count {
            let d = self.dirs[i];
            let hit = world.raycast(
                origin,
                {
                    let w = rot.apply(d);
                    Vec3 {
                        x: w[0],
                        y: w[1],
                        z: w[2],
                    }
                },
                max_m,
            );
            let noise = self.rng.gauss();
            let dropout = self.rng.chance(self.err.dropout);
            let (mm, st) = match hit {
                Some(h) if !dropout && !blind => {
                    let sigma = self.err.sigma_base_mm + self.err.sigma_per_m_mm * h.dist;
                    let mm = (h.dist * 1000.0 + noise * sigma).round().max(0.0);
                    if mm > max_mm {
                        (TOF_OUT_OF_RANGE_MM, tof_status::NO_TARGET)
                    } else {
                        (mm as u16, tof_status::OK)
                    }
                }
                _ => (TOF_OUT_OF_RANGE_MM, tof_status::NO_TARGET),
            };
            self.zones[i] = mm;
            self.status[i] = st;
        }
        if ghost && !blind {
            self.zones[ghost_zone] = ghost_mm.round() as u16;
            self.status[ghost_zone] = tof_status::SIGMA_FAIL;
        }
        self.fresh = true;
        self.frame_pending = true;
    }

    fn reading(&self, fresh: bool) -> TofReading {
        TofReading {
            sensor_id: self.mount.id.clone(),
            zones: self.zones.clone(),
            status: self.status.clone(),
            fresh,
        }
    }
}

/// All ToF sensors of the robot.
pub struct TofArray {
    sensors: Vec<TofSensor>,
}

impl TofArray {
    pub fn new(robot: &RobotConfig, err: &TofErrors, rng: &SimRng) -> Self {
        TofArray {
            sensors: robot
                .tof
                .iter()
                .map(|m| TofSensor::new(m, err, rng))
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

    /// Readings for the controller; `fresh` is true once per new sample.
    pub fn observe(&mut self) -> Vec<TofReading> {
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
    pub fn drain_frame(&mut self) -> Vec<TofReading> {
        self.sensors
            .iter_mut()
            .filter(|s| s.frame_pending)
            .map(|s| {
                s.frame_pending = false;
                s.reading(true)
            })
            .collect()
    }

    /// Same as `drain_frame` without consuming.
    pub fn peek_frame(&self) -> Vec<TofReading> {
        self.sensors
            .iter()
            .filter(|s| s.frame_pending)
            .map(|s| s.reading(true))
            .collect()
    }
}
