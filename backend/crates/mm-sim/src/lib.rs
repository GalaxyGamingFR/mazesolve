//! MazeMuse simulator: deterministic, fixed-timestep, seeded. See `docs/REPLAY_FORMAT.md`.

pub mod game;
pub mod kinematics;
pub mod replay;
pub mod rng;
pub mod sensors;
pub mod sim;
pub mod world;

pub use game::{GameCtx, GameState};
pub use kinematics::{BodyState, KinematicBackend, PhysicsBackend};
pub use replay::{config_hash, decode_msgpack, encode_msgpack, to_json, ReplayError};
pub use rng::{trng_seed, SimRng};
pub use sim::{ConfigHashes, Sim, SimError, SimInput, StepOutcome};
pub use world::{Contact, Hit, HitKind, World};

#[cfg(test)]
mod tests;
