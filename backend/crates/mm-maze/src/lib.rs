//! MazeMuse maze generator and validator.
//!
//! Conventions (also documented in `generate`):
//! * Tile `(col,row)` covers world `[col*t,(col+1)*t] x [row*t,(row+1)*t]`; north is +y.
//! * One `Wall` per physical tile edge. Edges are canonicalised to the south/west tile
//!   of the pair: an internal edge between `A` and its north/east neighbour is stored as
//!   `(A, North)` / `(A, East)`. Only perimeter edges use `South` (row 0) and `West`
//!   (col 0). `mm_core::MazeGraph::has_wall` understands both views.
//! * Black tiles are virtual walls: no `Wall` segment is stored for their edges.

mod ascii;
mod generate;
mod grid;
mod validate;

#[cfg(test)]
mod tests;

pub use ascii::ascii;
pub use generate::{generate, generate_with_report};
pub use grid::classify;
pub use validate::validate;

/// Errors from the generator.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MazeError {
    /// The request itself is invalid (grid size, out-of-range shares).
    #[error("bad request: {0}")]
    BadRequest(String),
    /// The generator produced a maze that its own validator rejects (a bug).
    #[error("generator produced an invalid maze: {0}")]
    Internal(String),
}
