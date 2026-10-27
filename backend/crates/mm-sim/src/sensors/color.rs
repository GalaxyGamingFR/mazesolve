//! AS7343 colour sensor. Stub: M4 fills in floor-colour readings.

use mm_core::{ColorReading, RobotConfig};

#[derive(Debug, Clone, Default)]
pub struct ColorArray;

impl ColorArray {
    // M4: build one sensor per `robot.color` mount with its own rng stream.
    pub fn new(_robot: &RobotConfig) -> Self {
        ColorArray
    }

    // M4: return the latest sample per sensor (fresh flag set once).
    pub fn observe(&mut self) -> Vec<ColorReading> {
        Vec::new()
    }

    // M4: samples not yet written to a replay frame.
    pub fn drain_frame(&mut self) -> Vec<ColorReading> {
        Vec::new()
    }
}
