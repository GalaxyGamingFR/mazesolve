//! OpenMV N6 camera: victim and decoy detections.
//!
//! A camera frame is taken every `1/rate_hz`. A target is detected when it is within
//! `max_range_m`, inside the horizontal and vertical field of view, in line of sight (a
//! raycast to the target finds no wall or obstacle first) and facing the camera. The result
//! is then degraded by the `CameraErrors` (recall, blur, class and ring confusion, decoy
//! confusion, ghost detections) and delivered `latency_ms` after capture.

use std::collections::VecDeque;

use mm_core::{
    CamDetection, CameraErrors, CameraMount, DetectionKind, RingColor, RobotConfig, Vec3,
    VictimKind, VictimStatus,
};

use crate::kinematics::BodyState;
use crate::rng::{SimRng, Stream};
use crate::world::{Rot3, World};

/// Distance noise (fraction of range) and bearing noise (rad) of the detector, applied
/// whenever the error profile is not the ideal one.
const DIST_SIGMA_FRAC: f64 = 0.03;
const BEARING_SIGMA_RAD: f64 = 0.01;
/// Extra clearance before the target when testing line of sight, on top of wall tolerance.
const LOS_CLEARANCE_M: f64 = 0.01;
const STATUSES: [VictimStatus; 3] = [
    VictimStatus::Harmed,
    VictimStatus::Stable,
    VictimStatus::Unharmed,
];
const RING_COLORS: [RingColor; 5] = [
    RingColor::Black,
    RingColor::Red,
    RingColor::Yellow,
    RingColor::Green,
    RingColor::Blue,
];

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

struct CameraSensor {
    mount: CameraMount,
    err: CameraErrors,
    rng: Stream,
    period_idx: u64,
    /// (ready time, detection), in capture order.
    pending: VecDeque<(f64, CamDetection)>,
    frame_pending: Vec<CamDetection>,
}

impl CameraSensor {
    fn new(mount: &CameraMount, err: &CameraErrors, rng: &SimRng) -> Self {
        CameraSensor {
            mount: mount.clone(),
            err: err.clone(),
            rng: rng.stream(&format!("camera/{}", mount.id)),
            period_idx: 0,
            pending: VecDeque::new(),
            frame_pending: Vec::new(),
        }
    }

    fn due(&self, t: f64) -> bool {
        t >= self.period_idx as f64 / self.mount.rate_hz - 1e-9
    }

    fn ideal(&self) -> bool {
        let e = &self.err;
        e.recall >= 1.0
            && e.false_positive <= 0.0
            && e.decoy_confusion <= 0.0
            && e.class_confusion <= 0.0
            && e.ring_confusion <= 0.0
    }

    fn other_status(&mut self, truth: VictimStatus) -> VictimStatus {
        let others: Vec<VictimStatus> = STATUSES.iter().copied().filter(|s| *s != truth).collect();
        others[self.rng.below(others.len())]
    }

    fn other_ring(&mut self, c: RingColor) -> RingColor {
        let others: Vec<RingColor> = RING_COLORS.iter().copied().filter(|r| *r != c).collect();
        others[self.rng.below(others.len())]
    }

    fn sample(&mut self, t: f64, body: &BodyState, world: &World) {
        self.period_idx = (t * self.mount.rate_hz + 1e-9).floor() as u64 + 1;
        let rot = Rot3::from_ypr(body.yaw, body.pitch, body.roll);
        let p = self.mount.pose;
        let lo = rot.apply([p.x, p.y, p.z]);
        let origin = Vec3 {
            x: body.x + lo[0],
            y: body.y + lo[1],
            z: body.z + lo[2],
        };
        let mrot = Rot3::from_ypr(p.yaw, p.pitch, 0.0);
        let fwd = rot.apply(mrot.apply([1.0, 0.0, 0.0]));
        let left = rot.apply(mrot.apply([0.0, 1.0, 0.0]));
        let up = rot.apply(mrot.apply([0.0, 0.0, 1.0]));
        let recall = if body.omega.abs() > self.err.blur_omega {
            self.err.blur_recall
        } else {
            self.err.recall
        };
        let noisy = !self.ideal();
        let ready = t + self.err.latency_ms / 1000.0;
        let half_h = self.mount.hfov_deg.to_radians() / 2.0;
        let half_v = self.mount.vfov_deg.to_radians() / 2.0;
        let max_r = self.mount.max_range_m;
        let los_eps = world.applied_tolerance * world.tile_m + LOS_CLEARANCE_M;
        let mut produced = 0;
        for v in &world.maze.victims {
            let d = [v.pos.x - origin.x, v.pos.y - origin.y, v.pos.z - origin.z];
            let dist = dot3(d, d).sqrt();
            if !(1e-6..=max_r).contains(&dist) {
                continue;
            }
            let x = dot3(d, fwd);
            if x <= 1e-6 {
                continue;
            }
            let bearing = dot3(d, left).atan2(x);
            let elev = dot3(d, up).atan2(x);
            if bearing.abs() > half_h || elev.abs() > half_v {
                continue;
            }
            // Facing: the target's outward normal must point toward the camera.
            let (sf, cf) = v.facing_yaw.sin_cos();
            if cf * d[0] + sf * d[1] >= 0.0 {
                continue;
            }
            if dist > los_eps
                && world
                    .raycast(
                        origin,
                        Vec3 {
                            x: d[0],
                            y: d[1],
                            z: d[2],
                        },
                        dist - los_eps,
                    )
                    .is_some()
            {
                continue;
            }
            // Fixed draw count per visible target.
            let seen = self.rng.chance(recall);
            let class_wrong = self.rng.chance(self.err.class_confusion);
            let ring_wrong: Vec<bool> = (0..5)
                .map(|_| self.rng.chance(self.err.ring_confusion))
                .collect();
            let n_dist = self.rng.gauss();
            let n_bear = self.rng.gauss();
            let n_conf = self.rng.gauss();
            let decoy_seen = self.rng.chance(self.err.decoy_confusion);
            let decoy_status = STATUSES[self.rng.below(STATUSES.len())];
            if !seen {
                continue;
            }
            let (kind, status, rings) = match &v.kind {
                VictimKind::Smiley { status } => {
                    let s = if class_wrong {
                        self.other_status(*status)
                    } else {
                        *status
                    };
                    (DetectionKind::Smiley, Some(s), None)
                }
                VictimKind::Cognitive { rings } => {
                    let mut r = *rings;
                    for (i, wrong) in ring_wrong.iter().enumerate() {
                        if *wrong {
                            r[i] = self.other_ring(r[i]);
                        }
                    }
                    (DetectionKind::Cognitive, None, Some(r))
                }
                VictimKind::Decoy => {
                    if !decoy_seen {
                        continue;
                    }
                    (DetectionKind::Smiley, Some(decoy_status), None)
                }
            };
            let off_axis = bearing.abs() / half_h.max(1e-6);
            let confidence = (0.98 - 0.4 * dist / max_r - 0.2 * off_axis
                + if noisy { 0.02 * n_conf } else { 0.0 })
            .clamp(0.05, 1.0);
            let (dist_m, bear) = if noisy {
                (
                    (dist * (1.0 + DIST_SIGMA_FRAC * n_dist)).max(0.0),
                    bearing + BEARING_SIGMA_RAD * n_bear,
                )
            } else {
                (dist, bearing)
            };
            produced += 1;
            self.pending.push_back((
                ready,
                CamDetection {
                    camera_id: self.mount.id.clone(),
                    kind,
                    status,
                    rings,
                    confidence,
                    distance_m: dist_m,
                    bearing: bear,
                    captured_t: t,
                },
            ));
        }
        // Ghost detection when nothing real is in view.
        let ghost = self.rng.chance(self.err.false_positive);
        let g_status = STATUSES[self.rng.below(STATUSES.len())];
        let g_dist = self.rng.range(0.1 * max_r, max_r);
        let g_bear = self.rng.range(-half_h, half_h);
        let g_conf = self.rng.range(0.2, 0.5);
        if ghost && produced == 0 {
            self.pending.push_back((
                ready,
                CamDetection {
                    camera_id: self.mount.id.clone(),
                    kind: DetectionKind::Smiley,
                    status: Some(g_status),
                    rings: None,
                    confidence: g_conf,
                    distance_m: g_dist,
                    bearing: g_bear,
                    captured_t: t,
                },
            ));
        }
    }

    fn release(&mut self, t: f64) -> Vec<CamDetection> {
        let mut out = Vec::new();
        while self.pending.front().is_some_and(|(r, _)| *r <= t + 1e-9) {
            if let Some((_, d)) = self.pending.pop_front() {
                self.frame_pending.push(d.clone());
                out.push(d);
            }
        }
        out
    }
}

/// All cameras of the robot.
pub struct CameraArray {
    sensors: Vec<CameraSensor>,
}

impl CameraArray {
    pub fn new(robot: &RobotConfig, err: &CameraErrors, rng: &SimRng) -> Self {
        CameraArray {
            sensors: robot
                .cameras
                .iter()
                .map(|m| CameraSensor::new(m, err, rng))
                .collect(),
        }
    }

    /// Capture a frame on every camera whose period has elapsed at sim time `t`.
    pub fn sample_due(&mut self, t: f64, body: &BodyState, world: &World) {
        for s in &mut self.sensors {
            if s.due(t) {
                s.sample(t, body, world);
            }
        }
    }

    /// Detections whose latency has elapsed by sim time `t`.
    pub fn observe(&mut self, t: f64) -> Vec<CamDetection> {
        self.sensors.iter_mut().flat_map(|s| s.release(t)).collect()
    }

    /// Detections delivered since the previous frame, consumed.
    pub fn drain_frame(&mut self) -> Vec<CamDetection> {
        self.sensors
            .iter_mut()
            .flat_map(|s| std::mem::take(&mut s.frame_pending))
            .collect()
    }
}
