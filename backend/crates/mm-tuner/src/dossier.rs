//! Compact text dossier of one run for Muse. Target 30k tokens (chars / 4), hard-capped.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use mm_core::{
    tof_status, EndReason, Event, EventKind, Frame, Metrics, Replay, TunableParam,
    TOF_OUT_OF_RANGE_MM,
};

pub const MAX_DOSSIER_TOKENS: usize = 30_000;
const MAX_CHARS: usize = MAX_DOSSIER_TOKENS * 4;
const MAX_CODE_CHARS: usize = 6_000;
const MAX_DETAIL_ITEMS: usize = 60;
const MAX_CLUSTER_ITEMS: usize = 20;
const TRUNC_MARKER: &str = "\n[... dossier truncated at token cap ...]\n";

pub struct DossierInput<'a> {
    pub replay: &'a Replay,
    pub metrics: &'a Metrics,
    pub params: &'a [TunableParam],
    pub current: &'a BTreeMap<String, f64>,
    pub controller_code: Option<&'a str>,
    pub robot_summary: String,
    pub error_profile_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dossier {
    pub text: String,
    pub truncated: bool,
}

impl Dossier {
    /// chars / 4, rounded up.
    pub fn estimated_tokens(&self) -> usize {
        self.text.chars().count().div_ceil(4)
    }
}

fn kind_name(k: &EventKind) -> String {
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v["type"].as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into())
}

fn is_failure(k: &EventKind) -> bool {
    match k {
        EventKind::LopRestart { .. }
        | EventKind::Misidentification { .. }
        | EventKind::DecoyFalsePositive { .. }
        | EventKind::VictimMissed { .. }
        | EventKind::EnteredBlack { .. }
        | EventKind::ControllerError { .. } => true,
        EventKind::BlueStop { ok, .. } => !ok,
        _ => false,
    }
}

/// Sensor/pose summary for frames within +-2 s of `t`.
fn window_summary(frames: &[Frame], t: f64) -> String {
    let lo = frames.partition_point(|f| f.t < t - 2.0);
    let hi = frames.partition_point(|f| f.t <= t + 2.0);
    let w = &frames[lo..hi.max(lo)];
    if w.is_empty() {
        return "no frames".into();
    }
    let mut min_front = u16::MAX;
    let mut min_any = u16::MAX;
    let mut max_gyro = 0.0f64;
    let mut max_tilt = 0.0f64;
    let mut path = 0.0f64;
    let mut nearest = &w[0];
    for (i, f) in w.iter().enumerate() {
        if (f.t - t).abs() < (nearest.t - t).abs() {
            nearest = f;
        }
        if i > 0 {
            let p = &w[i - 1].pose;
            path += ((f.pose.x - p.x).powi(2) + (f.pose.y - p.y).powi(2)).sqrt();
        }
        max_gyro = max_gyro.max(f.imu.gyro.iter().fold(0.0f64, |a, g| a.max(g.abs())));
        max_tilt = max_tilt.max(f.pose.roll.abs()).max(f.pose.pitch.abs());
        for r in &f.tof {
            let front = r.sensor_id.to_lowercase().contains("front");
            for (z, s) in r.zones.iter().zip(r.status.iter()) {
                if *s == tof_status::OK && *z != TOF_OUT_OF_RANGE_MM {
                    min_any = min_any.min(*z);
                    if front {
                        min_front = min_front.min(*z);
                    }
                }
            }
        }
    }
    let span = (w[w.len() - 1].t - w[0].t).max(1e-9);
    let fmt_mm = |v: u16| {
        if v == u16::MAX {
            "n/a".to_string()
        } else {
            format!("{v}mm")
        }
    };
    format!(
        "pose=({:.2},{:.2}) yaw={:.2} speed~{:.2}m/s min_front_tof={} min_any_tof={} max_gyro={:.2}rad/s max_tilt={:.2}rad",
        nearest.pose.x,
        nearest.pose.y,
        nearest.pose.yaw,
        path / span,
        fmt_mm(min_front),
        fmt_mm(min_any),
        max_gyro,
        max_tilt
    )
}

fn describe(e: &Event) -> String {
    match &e.kind {
        EventKind::LopRestart { to, restarts } => {
            format!(
                "LopRestart #{restarts} back to tile ({},{})",
                to.col, to.row
            )
        }
        EventKind::Misidentification { reported } => {
            format!("Misidentification reported {reported:?}")
        }
        EventKind::DecoyFalsePositive { victim_id } => format!("DecoyFalsePositive id={victim_id}"),
        EventKind::VictimMissed { victim_id } => format!("VictimMissed id={victim_id}"),
        EventKind::EnteredBlack { tile } => {
            format!("EnteredBlack tile ({},{})", tile.col, tile.row)
        }
        EventKind::ControllerError { message } => {
            format!(
                "ControllerError: {}",
                message.chars().take(200).collect::<String>()
            )
        }
        EventKind::BlueStop { tile, .. } => {
            format!("BlueStop FAILED at tile ({},{})", tile.col, tile.row)
        }
        other => kind_name(other),
    }
}

struct Cluster {
    t0: f64,
    t1: f64,
    n: usize,
    x: f64,
    y: f64,
}

fn collision_clusters(events: &[Event]) -> Vec<Cluster> {
    let mut out: Vec<Cluster> = Vec::new();
    for e in events {
        if let EventKind::Collision { x, y } = e.kind {
            match out.last_mut() {
                Some(c) if e.t - c.t1 <= 1.0 => {
                    c.t1 = e.t;
                    c.n += 1;
                }
                _ => out.push(Cluster {
                    t0: e.t,
                    t1: e.t,
                    n: 1,
                    x,
                    y,
                }),
            }
        }
    }
    out
}

fn code_excerpt(code: &str) -> String {
    if code.len() < 4000 {
        return code.to_string();
    }
    let lines: Vec<&str> = code.lines().collect();
    let mut keep = vec![false; lines.len()];
    for (i, l) in lines.iter().enumerate() {
        if l.contains("@tunable") {
            let end = (i + 2).min(lines.len());
            keep[i.saturating_sub(1)..end].fill(true);
        }
        if l.contains("fn step") {
            let end = (i + 60).min(lines.len());
            keep[i.saturating_sub(2)..end].fill(true);
        }
    }
    let mut out = String::new();
    let mut gap = false;
    for (i, l) in lines.iter().enumerate() {
        if keep[i] {
            let _ = writeln!(out, "{:>4}| {l}", i + 1);
            gap = false;
        } else if !gap {
            out.push_str("   ...\n");
            gap = true;
        }
        if out.len() > MAX_CODE_CHARS {
            out.push_str("   ... [code excerpt truncated]\n");
            break;
        }
    }
    out
}

pub fn build_dossier(input: &DossierInput) -> Dossier {
    let r = input.replay;
    let m = input.metrics;
    let h = &r.header;
    let mut s = String::new();

    let _ = writeln!(s, "# MazeMuse run dossier");
    let _ = writeln!(s, "\n## Config");
    let _ = writeln!(
        s,
        "run_id={} seed={} rng_mode={:?} error_profile={} tolerance_frac={:.3}",
        h.run_id, h.seed, h.rng_mode, input.error_profile_id, h.applied_tolerance_frac
    );
    let _ = writeln!(s, "robot: {}", input.robot_summary);
    let _ = writeln!(
        s,
        "duration={:.1}s exited={} frames={} events={}",
        m.duration_s,
        m.exited,
        r.frames.len(),
        r.events.len()
    );
    let end = r.events.iter().rev().find_map(|e| match &e.kind {
        EventKind::RunEnd { reason, .. } => Some(*reason),
        _ => None,
    });
    if let Some(reason) = end {
        let _ = writeln!(s, "end_reason={reason:?}");
    }

    // Score.
    let sc = &m.score;
    let _ = writeln!(s, "\n## Score");
    let _ = writeln!(
        s,
        "total={} victims={} kits={} checkpoints={} ramps={} stairs={} speed_bumps={} blue_tiles={} reliability={} exit_bonus={} penalties={}",
        sc.total, sc.victims, sc.kits, sc.checkpoints, sc.ramps, sc.stairs, sc.speed_bumps,
        sc.blue_tiles, sc.reliability, sc.exit_bonus, sc.penalties
    );
    let missed = r
        .events
        .iter()
        .filter(|e| matches!(e.kind, EventKind::VictimMissed { .. }))
        .count();
    let per_victim = if m.victims_identified > 0 {
        sc.victims as f64 / m.victims_identified as f64
    } else {
        0.0
    };
    let _ = writeln!(
        s,
        "victims: {}/{} identified, {} missed, {} misidentifications, {} decoy false positives, kits_used={}",
        m.victims_identified, m.victims_total, missed, m.misidentifications, m.decoy_false_positives, m.kits_used
    );
    let _ = writeln!(
        s,
        "headroom (estimate from events): missed victims ~{:.0} pts at {:.1} pts/identified victim; penalties cost {}; exit bonus {}",
        missed as f64 * per_victim,
        per_victim,
        -sc.penalties,
        if m.exited || end == Some(EndReason::Exited) {
            "earned"
        } else {
            "NOT earned (missed exit)"
        }
    );

    // Coverage and collisions.
    let clusters = collision_clusters(&r.events);
    let _ = writeln!(s, "\n## Coverage and collisions");
    let mut seen: HashSet<(u32, u32)> = HashSet::new();
    let mut last_new = 0.0;
    for e in &r.events {
        if let EventKind::TileEnter { tile } = e.kind {
            if seen.insert((tile.col, tile.row)) {
                last_new = e.t;
            }
        }
    }
    let _ = writeln!(
        s,
        "coverage={:.1}% ({}/{} tiles), last new tile at t={:.1}s of {:.1}s, distance={:.1}m, lop_restarts={}",
        m.coverage * 100.0,
        m.tiles_visited,
        m.tiles_reachable,
        last_new,
        m.duration_s,
        m.distance_m,
        m.restarts
    );
    let _ = writeln!(
        s,
        "collisions={} in {} clusters (events within 1s merged)",
        m.collisions,
        clusters.len()
    );
    let mut hot: BTreeMap<(i64, i64), usize> = BTreeMap::new();
    for e in &r.events {
        if let EventKind::Collision { x, y } = e.kind {
            *hot.entry(((x * 5.0).floor() as i64, (y * 5.0).floor() as i64))
                .or_default() += 1;
        }
    }
    let mut hot: Vec<_> = hot.into_iter().collect();
    hot.sort_by_key(|h| std::cmp::Reverse(h.1));
    for ((cx, cy), n) in hot.iter().take(5) {
        let _ = writeln!(
            s,
            "  hotspot x[{:.1},{:.1}) y[{:.1},{:.1}): {n} collisions",
            *cx as f64 / 5.0,
            (*cx + 1) as f64 / 5.0,
            *cy as f64 / 5.0,
            (*cy + 1) as f64 / 5.0
        );
    }

    // Event counts.
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in &r.events {
        *counts.entry(kind_name(&e.kind)).or_default() += 1;
    }
    let _ = writeln!(s, "\n## Event counts");
    let _ = writeln!(
        s,
        "{}",
        counts
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    );

    // Failure timeline.
    let _ = writeln!(s, "\n## Failure timeline (window = +-2s of frames)");
    let mut items: Vec<(f64, String)> = Vec::new();
    let fails: Vec<&Event> = r.events.iter().filter(|e| is_failure(&e.kind)).collect();
    for e in fails.iter().take(MAX_DETAIL_ITEMS) {
        items.push((
            e.t,
            format!(
                "t={:.2}s {} | {}",
                e.t,
                describe(e),
                window_summary(&r.frames, e.t)
            ),
        ));
    }
    let mut big: Vec<&Cluster> = clusters.iter().collect();
    big.sort_by_key(|c| std::cmp::Reverse(c.n));
    for c in big.into_iter().take(MAX_CLUSTER_ITEMS) {
        items.push((
            c.t0,
            format!(
                "t={:.2}s Collision cluster x{} until t={:.2}s near ({:.2},{:.2}) | {}",
                c.t0,
                c.n,
                c.t1,
                c.x,
                c.y,
                window_summary(&r.frames, c.t0)
            ),
        ));
    }
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, line) in &items {
        let _ = writeln!(s, "{line}");
    }
    if fails.len() > MAX_DETAIL_ITEMS {
        let _ = writeln!(
            s,
            "... {} more failure events omitted",
            fails.len() - MAX_DETAIL_ITEMS
        );
    }
    if clusters.len() > MAX_CLUSTER_ITEMS {
        let _ = writeln!(
            s,
            "... {} smaller collision clusters omitted",
            clusters.len() - MAX_CLUSTER_ITEMS
        );
    }
    if items.is_empty() {
        let _ = writeln!(s, "(no failure events)");
    }
    if !m.exited && end != Some(EndReason::Exited) {
        let last = r.frames.last();
        let _ = writeln!(
            s,
            "MISSED EXIT: run did not exit (end_reason={}). Final state: {}",
            end.map_or("none".to_string(), |e| format!("{e:?}")),
            last.map_or("no frames".to_string(), |f| window_summary(&r.frames, f.t))
        );
    }

    // Params.
    let _ = writeln!(
        s,
        "\n## Tunable parameters (path | current | min | max | max_step | description)"
    );
    for p in input.params {
        let cur = input.current.get(&p.path).copied().unwrap_or(p.default);
        let _ = writeln!(
            s,
            "{} | {} | {} | {} | {} | {}",
            p.path, cur, p.min, p.max, p.max_step, p.description
        );
    }

    // Code.
    if let Some(code) = input.controller_code {
        let _ = writeln!(s, "\n## Controller code (excerpt)\n```rhai");
        s.push_str(&code_excerpt(code));
        if !s.ends_with('\n') {
            s.push('\n');
        }
        let _ = writeln!(s, "```");
    }

    let mut truncated = false;
    if s.chars().count() > MAX_CHARS {
        let keep = MAX_CHARS - TRUNC_MARKER.len();
        s = s.chars().take(keep).collect();
        s.push_str(TRUNC_MARKER);
        truncated = true;
    }
    Dossier { text: s, truncated }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use mm_core::{
        ImuReading, Pose3, ReplayHeader, RngMode, ScoreBreakdown, TileXY, TofReading, WheelState,
    };

    pub fn header() -> ReplayHeader {
        ReplayHeader {
            format_version: 1,
            run_id: "r1".into(),
            seed: 1,
            rng_mode: RngMode::Fixed,
            robot_hash: String::new(),
            maze_hash: String::new(),
            controller_hash: String::new(),
            error_profile_hash: String::new(),
            rules_hash: String::new(),
            applied_tolerance_frac: 0.0,
            physics_dt_s: 0.001,
            controller_dt_s: 0.01,
            frame_dt_s: 0.04,
            created_at: None,
            backend_version: "t".into(),
        }
    }

    pub fn frame(t: f64) -> Frame {
        Frame {
            t,
            pose: Pose3 {
                x: t * 0.1,
                y: 0.0,
                z: 0.0,
                roll: 0.0,
                pitch: 0.0,
                yaw: 0.0,
            },
            wheels: [WheelState::default(); 4],
            imu: ImuReading::default(),
            tof: vec![TofReading {
                sensor_id: "front".into(),
                zones: vec![120, 300],
                status: vec![5, 5],
                fresh: true,
            }],
            color: vec![],
            cams: vec![],
            id_light: false,
        }
    }

    pub fn small_replay() -> Replay {
        Replay {
            header: header(),
            frames: (0..100).map(|i| frame(i as f64 * 0.04)).collect(),
            events: vec![],
        }
    }

    fn params() -> Vec<TunableParam> {
        vec![TunableParam {
            path: "controller.wall_dist".into(),
            description: "target wall distance".into(),
            min: 0.05,
            max: 0.3,
            default: 0.2,
            max_step: 0.03,
        }]
    }

    #[test]
    fn big_replay_stays_under_cap() {
        let frames: Vec<Frame> = (0..10_000).map(|i| frame(i as f64 * 0.04)).collect();
        let mut events = Vec::new();
        for i in 0..2000u32 {
            let t = i as f64 * 0.19;
            let kind = match i % 5 {
                0 => EventKind::Collision {
                    x: i as f64 * 0.37 % 5.0,
                    y: i as f64 * 0.11 % 5.0,
                },
                1 => EventKind::LopRestart {
                    to: TileXY { col: 1, row: 1 },
                    restarts: i,
                },
                2 => EventKind::VictimMissed { victim_id: i },
                3 => EventKind::TileEnter {
                    tile: TileXY {
                        col: i % 9,
                        row: i % 7,
                    },
                },
                _ => EventKind::ControllerError {
                    message: "x".repeat(500),
                },
            };
            events.push(Event { t, kind });
        }
        events.push(Event {
            t: 399.0,
            kind: EventKind::RunEnd {
                reason: EndReason::TimeUp,
                score: ScoreBreakdown::default(),
            },
        });
        let replay = Replay {
            header: header(),
            frames,
            events,
        };
        let metrics = Metrics::default();
        let current = BTreeMap::new();
        let ps = params();
        let code = "let x = 1; // @tunable\n".repeat(1000);
        let d = build_dossier(&DossierInput {
            replay: &replay,
            metrics: &metrics,
            params: &ps,
            current: &current,
            controller_code: Some(&code),
            robot_summary: "4wd".into(),
            error_profile_id: "realistic".into(),
        });
        assert!(
            d.estimated_tokens() <= MAX_DOSSIER_TOKENS,
            "{}",
            d.estimated_tokens()
        );
        assert!(d.text.contains("MISSED EXIT"));
        assert!(d.text.contains("controller.wall_dist"));
        assert!(d.text.contains("min_front_tof=120mm"));
    }

    #[test]
    fn truncation_marker_when_over_cap() {
        let replay = small_replay();
        let metrics = Metrics::default();
        let current = BTreeMap::new();
        let ps = params();
        let d = build_dossier(&DossierInput {
            replay: &replay,
            metrics: &metrics,
            params: &ps,
            current: &current,
            controller_code: None,
            robot_summary: "x".repeat(MAX_CHARS + 10),
            error_profile_id: "clean".into(),
        });
        assert!(d.truncated);
        assert!(d.text.contains("truncated"));
        assert!(d.estimated_tokens() <= MAX_DOSSIER_TOKENS);
    }

    #[test]
    fn short_code_is_whole_long_code_is_excerpted() {
        assert_eq!(code_excerpt("let a = 1;"), "let a = 1;");
        let mut long = String::new();
        for i in 0..400 {
            long.push_str(&format!("let v{i} = {i};\n"));
        }
        long.push_str("// @tunable wall_dist\nfn step(o) {\n  o\n}\n");
        let e = code_excerpt(&long);
        assert!(e.contains("fn step") && e.contains("@tunable") && e.contains("..."));
        assert!(!e.contains("let v5 ="));
    }
}
