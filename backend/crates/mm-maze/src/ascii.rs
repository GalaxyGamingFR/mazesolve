//! Debug renderer.

use mm_core::{Dir, MazeGraph, TileFeature, TileKind, TileXY};

use crate::grid::WallSet;

/// ASCII picture, north at the top. Walls are `+--+` / `|`. Each tile shows two
/// characters: a kind letter (S start, B blue, X black, U silver, R red entrance) or a
/// feature letter (r ramp, t stairs, b bump, o obstacle), then `d` inside the Dangerous
/// Zone or `v` when a victim sits on one of its walls.
pub fn ascii(m: &MazeGraph) -> String {
    let ws = WallSet::from_graph(m);
    let (w, h) = (m.grid.w, m.grid.h);
    let mut out = String::new();
    let hline = |out: &mut String, row: u32, side: Dir| {
        for col in 0..w {
            out.push('+');
            out.push_str(if ws.has(TileXY::new(col, row), side) {
                "--"
            } else {
                "  "
            });
        }
        out.push_str("+\n");
    };
    for row in (0..h).rev() {
        hline(&mut out, row, Dir::North);
        for col in 0..w {
            let at = TileXY::new(col, row);
            out.push(if ws.has(at, Dir::West) { '|' } else { ' ' });
            let Some(t) = m.tile(at) else { continue };
            let c = match t.kind {
                TileKind::Start => 'S',
                TileKind::Blue => 'B',
                TileKind::Black => 'X',
                TileKind::Silver => 'U',
                TileKind::Red => 'R',
                TileKind::Normal => match t.features.first() {
                    Some(TileFeature::Ramp { .. }) => 'r',
                    Some(TileFeature::Stairs { .. }) => 't',
                    Some(TileFeature::SpeedBump { .. }) => 'b',
                    None if m.obstacles.iter().any(|o| o.tile == at) => 'o',
                    None => ' ',
                },
            };
            out.push(c);
            out.push(if t.in_dz {
                'd'
            } else if m.victims.iter().any(|v| v.tile == at) {
                'v'
            } else {
                ' '
            });
        }
        out.push(if ws.has(TileXY::new(w - 1, row), Dir::East) {
            '|'
        } else {
            ' '
        });
        out.push('\n');
    }
    hline(&mut out, 0, Dir::South);
    out
}
