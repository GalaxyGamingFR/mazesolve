//! OpenMV N6 camera detections. Stub: M4 fills in victim detection.

use mm_core::{CamDetection, RobotConfig};

#[derive(Debug, Clone, Default)]
pub struct CameraArray;

impl CameraArray {
    // M4: one camera model per `robot.cameras` mount with latency queue.
    pub fn new(_robot: &RobotConfig) -> Self {
        CameraArray
    }

    // M4: detections that have cleared their latency this step.
    pub fn observe(&mut self) -> Vec<CamDetection> {
        Vec::new()
    }

    // M4: detections not yet written to a replay frame.
    pub fn drain_frame(&mut self) -> Vec<CamDetection> {
        Vec::new()
    }
}
