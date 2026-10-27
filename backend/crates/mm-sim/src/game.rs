//! Rules-level game state: scoring and run bookkeeping.
//!
//! M3 scores checkpoints only. M4 fills in the hook points below (victims, kits, blue
//! tiles, ramps/stairs/bumps, exit bonus, reliability). Every hook receives a `GameCtx`
//! with the world and ground truth, and pushes `EventKind`s that the sim time-stamps.

use std::collections::BTreeSet;

use mm_core::{
    Commands, EndReason, EventKind, MazeGraph, Pose3, RobotConfig, RulesProfile, ScoreBreakdown,
    TileKind, TileXY,
};

/// Read-only context for the hooks.
pub struct GameCtx<'a> {
    pub t: f64,
    pub rules: &'a RulesProfile,
    pub maze: &'a MazeGraph,
    /// Ground-truth pose.
    pub pose: Pose3,
    /// Ground-truth speed, m/s.
    pub speed: f64,
}

#[derive(Debug, Clone)]
pub struct GameState {
    pub score: ScoreBreakdown,
    pub kits_left: u32,
    /// Last silver tile visited (LoP teleports here).
    pub last_checkpoint: Option<TileXY>,
    pub lop_count: u32,
    checkpoints_seen: BTreeSet<TileXY>,
}

impl GameState {
    pub fn new(_rules: &RulesProfile, robot: &RobotConfig) -> Self {
        GameState {
            score: ScoreBreakdown::default(),
            kits_left: robot.kits,
            last_checkpoint: None,
            lop_count: 0,
            checkpoints_seen: BTreeSet::new(),
        }
    }

    /// Called once whenever the robot centre enters a tile (not for LoP teleports).
    /// M4: blue-stop timer start, ramp/stairs/bump traversal, black handling, DZ entry.
    pub fn on_tile_enter(
        &mut self,
        ctx: &GameCtx,
        tile: TileXY,
        kind: TileKind,
        out: &mut Vec<EventKind>,
    ) {
        if kind == TileKind::Silver && self.checkpoints_seen.insert(tile) {
            self.score.checkpoints += ctx.rules.scoring.checkpoint;
            self.last_checkpoint = Some(tile);
            out.push(EventKind::Checkpoint { tile });
        } else if kind == TileKind::Silver {
            self.last_checkpoint = Some(tile);
        }
    }

    /// Called every controller step with the commands just issued.
    /// M4: `report` (victim identification), `deploy_kits`, `id_light` blink timing,
    /// blue-tile stop timing, exit light.
    pub fn on_commands(&mut self, _ctx: &GameCtx, _cmds: &Commands, _out: &mut Vec<EventKind>) {}

    /// Called after a Lack-of-Progress restart (reliability penalty in M4).
    pub fn on_lop_restart(&mut self) {
        self.lop_count += 1;
    }

    /// Called once at run end. M4: reliability and exit bonuses, missed victims.
    pub fn finish(
        &mut self,
        _ctx: &GameCtx,
        _reason: EndReason,
        _out: &mut Vec<EventKind>,
    ) -> ScoreBreakdown {
        self.score_now()
    }

    /// Current score with the total summed.
    pub fn score_now(&self) -> ScoreBreakdown {
        let mut s = self.score.clone();
        s.sum();
        s
    }
}
