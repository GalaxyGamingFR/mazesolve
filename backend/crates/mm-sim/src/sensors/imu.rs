//! IMU models: IMX-5 tactical, BNO055 fused, raw MPU-class accel/gyro.

use mm_core::{ImuErrors, ImuPreset, ImuReading};

use crate::kinematics::BodyState;
use crate::rng::{SimRng, Stream};

const G: f64 = 9.80665;
/// Magnetic offset decays with this time constant (gone within ~2 s).
const MAG_TAU_S: f64 = 0.7;
const VIB_TAU_S: f64 = 0.2;

/// Wrap an angle to (-pi, pi].
pub fn wrap_pi(a: f64) -> f64 {
    use std::f64::consts::PI;
    let x = (a + PI).rem_euclid(2.0 * PI) - PI;
    if x <= -PI {
        PI
    } else {
        x
    }
}

pub struct Imu {
    preset: ImuPreset,
    scale: f64,
    extra_walk_deg_rthr: f64,
    mag_rate: f64,
    mag_rad: f64,
    vib_rad: f64,
    rng: Stream,
    /// In-run constant gyro bias drift (rad/s), IMX-5 only.
    bias_rate: f64,
    bias: f64,
    walk: f64,
    mag: f64,
    vib: [f64; 3],
    prev_v: f64,
    prev_roll: f64,
    prev_pitch: f64,
    reading: ImuReading,
}

impl Imu {
    pub fn new(preset: ImuPreset, e: &ImuErrors, rng: &SimRng) -> Self {
        let mut s = rng.stream("imu");
        let sign = s.sign();
        let bias_rate = match preset {
            ImuPreset::Imx5Tactical => sign * (1.5f64 / 3600.0).to_radians() * e.noise_scale,
            _ => 0.0,
        };
        Imu {
            preset,
            scale: e.noise_scale,
            extra_walk_deg_rthr: e.heading_walk_deg_rthr,
            mag_rate: e.mag_event_rate,
            mag_rad: e.mag_event_deg.to_radians(),
            vib_rad: e.vibration_deg.to_radians(),
            rng: s,
            bias_rate,
            bias: 0.0,
            walk: 0.0,
            mag: 0.0,
            vib: [0.0; 3],
            prev_v: 0.0,
            prev_roll: 0.0,
            prev_pitch: 0.0,
            reading: ImuReading::default(),
        }
    }

    /// Accumulated slow heading drift (bias + random walk), radians, without white noise.
    pub fn drift_rad(&self) -> f64 {
        self.bias + self.walk
    }

    pub fn reading(&self) -> ImuReading {
        self.reading
    }

    /// Advance by `dt` (0 is allowed for the initial reading) given the true body state.
    pub fn update(&mut self, dt: f64, b: &BodyState, jolt: bool) {
        let raw = self.preset == ImuPreset::RawImu;
        let walk_deg_rthr = match self.preset {
            ImuPreset::Bno055 => 0.5 * self.scale,
            _ => 0.0,
        } + self.extra_walk_deg_rthr;
        let g = self.rng.gauss();
        self.walk += g * (walk_deg_rthr / 60.0).to_radians() * dt.sqrt();
        self.bias += self.bias_rate * dt;

        let event = self.rng.chance(self.mag_rate * dt);
        let sign = self.rng.sign();
        if event {
            self.mag += sign * self.mag_rad;
        }
        self.mag *= (-dt / MAG_TAU_S).exp();

        if jolt {
            for k in 0..3 {
                self.vib[k] += self.rng.gauss() * self.vib_rad;
            }
        }
        let decay = (-dt / VIB_TAU_S).exp();
        for v in &mut self.vib {
            *v *= decay;
        }

        let n: Vec<f64> = (0..9).map(|_| self.rng.gauss()).collect();
        let (yaw_sig, rp_sig, gyro_sig, acc_sig) = match self.preset {
            ImuPreset::Imx5Tactical => (0.13f64.to_radians(), 0.05f64.to_radians(), 0.002, 0.02),
            ImuPreset::Bno055 => (1.0f64.to_radians(), 0.5f64.to_radians(), 0.002, 0.02),
            ImuPreset::RawImu => (0.0, 0.0, 0.01, 0.05),
        };
        let s = self.scale;
        let (roll_rate, pitch_rate, a_long) = if dt > 0.0 {
            (
                (b.roll - self.prev_roll) / dt,
                (b.pitch - self.prev_pitch) / dt,
                (b.v - self.prev_v) / dt,
            )
        } else {
            (0.0, 0.0, 0.0)
        };
        self.prev_roll = b.roll;
        self.prev_pitch = b.pitch;
        self.prev_v = b.v;

        let (yaw, roll, pitch) = if raw {
            (0.0, 0.0, 0.0)
        } else {
            (
                wrap_pi(
                    b.yaw + self.bias + self.walk + self.mag + self.vib[0] + n[0] * yaw_sig * s,
                ),
                b.roll + self.vib[1] + n[1] * rp_sig * s,
                b.pitch + self.vib[2] + n[2] * rp_sig * s,
            )
        };
        let (sp, cp) = b.pitch.sin_cos();
        let (sr, cr) = b.roll.sin_cos();
        self.reading = ImuReading {
            roll,
            pitch,
            yaw,
            gyro: [
                roll_rate + n[3] * gyro_sig * s,
                pitch_rate + n[4] * gyro_sig * s,
                b.omega + n[5] * gyro_sig * s,
            ],
            accel: [
                a_long + G * sp + n[6] * acc_sig * s,
                b.v * b.omega + G * sr + n[7] * acc_sig * s,
                G * cp * cr + n[8] * acc_sig * s,
            ],
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::ErrorProfile;

    fn rms_drift(preset: ImuPreset, secs: f64, errors: &ImuErrors) -> f64 {
        let b = BodyState::default();
        let n = 200;
        let sum: f64 = (0..n)
            .map(|seed| {
                let mut imu = Imu::new(preset, errors, &SimRng::new(seed));
                let steps = (secs / 0.01) as usize;
                for _ in 0..steps {
                    imu.update(0.01, &b, false);
                }
                imu.drift_rad().powi(2)
            })
            .sum();
        (sum / n as f64).sqrt()
    }

    #[test]
    fn bno055_drift_grows_over_a_minute() {
        let e = ErrorProfile::realistic().imu;
        let early = rms_drift(ImuPreset::Bno055, 5.0, &e);
        let late = rms_drift(ImuPreset::Bno055, 60.0, &e);
        assert!(late > 2.0 * early, "early {early} late {late}");
        assert!(late > 0.0);
    }

    #[test]
    fn clean_yaw_equals_truth_and_wraps() {
        let e = ErrorProfile::clean().imu;
        let mut imu = Imu::new(ImuPreset::Bno055, &e, &SimRng::new(1));
        let b = BodyState {
            yaw: 7.0,
            ..Default::default()
        };
        imu.update(0.01, &b, true);
        assert!((imu.reading().yaw - wrap_pi(7.0)).abs() < 1e-12);
        assert!(
            imu.reading().yaw <= std::f64::consts::PI && imu.reading().yaw > -std::f64::consts::PI
        );
    }

    #[test]
    fn raw_imu_has_zero_fused_angles_and_gravity() {
        let e = ErrorProfile::realistic().imu;
        let mut imu = Imu::new(ImuPreset::RawImu, &e, &SimRng::new(3));
        imu.update(0.01, &BodyState::default(), false);
        let r = imu.reading();
        assert_eq!((r.yaw, r.roll, r.pitch), (0.0, 0.0, 0.0));
        assert!((r.accel[2] - G).abs() < 0.5);
    }
}
