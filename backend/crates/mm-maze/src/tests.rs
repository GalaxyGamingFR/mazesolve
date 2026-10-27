use std::collections::BTreeSet;

use mm_core::{
    rules_2027_draft, Dir, FeatureCounts, GenerateRequest, GridSize, MazeGraph, RulesProfile,
    Severity, TileFeature, TileKind, TileXY, VictimKind, VictimTier,
};
use proptest::prelude::*;

use crate::grid::{idx, reach, WallSet};
use crate::{ascii, classify, generate, generate_with_report, validate, MazeError};

fn req(w: u32, h: u32, features: FeatureCounts) -> GenerateRequest {
    GenerateRequest {
        seed: None,
        grid: GridSize { w, h },
        rules: "rcj_maze_2027_draft".into(),
        features,
        name: None,
    }
}

fn none() -> FeatureCounts {
    FeatureCounts {
        black: 0,
        silver: 0,
        blue: 0,
        ramps: 0,
        stairs: 0,
        speed_bumps: 0,
        obstacles: 0,
        smiley_victims: 0,
        cognitive_victims: 0,
        decoys: 0,
        dangerous_zone: false,
        loopiness: 0.0,
        floating_victim_share: 0.3,
    }
}

fn gen(w: u32, h: u32, seed: u64, f: FeatureCounts) -> MazeGraph {
    generate(&req(w, h, f), seed, &rules_2027_draft()).expect("generates")
}

fn assert_ok(m: &MazeGraph, rules: &RulesProfile) {
    let v = validate(m, rules);
    assert!(v.ok, "{:#?}\n{}", v.issues, ascii(m));
}

fn errors(m: &MazeGraph, rules: &RulesProfile) -> Vec<String> {
    validate(m, rules)
        .issues
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.message)
        .collect()
}

#[test]
fn grid_limits() {
    let r = rules_2027_draft();
    for (w, h) in [(1, 5), (5, 1), (21, 5), (5, 21), (0, 0)] {
        let e = generate(&req(w, h, none()), 1, &r).unwrap_err();
        assert!(matches!(e, MazeError::BadRequest(_)), "{w}x{h}");
    }
    assert!(generate(&req(2, 2, none()), 1, &r).is_ok());
    assert!(generate(&req(20, 20, FeatureCounts::default()), 1, &r).is_ok());
    let mut f = none();
    f.loopiness = 1.5;
    assert!(generate(&req(4, 4, f), 1, &r).is_err());
}

#[test]
fn same_seed_same_maze() {
    let r = rules_2027_draft();
    let rq = req(9, 7, FeatureCounts::default());
    let a = generate(&rq, 42, &r).unwrap();
    let b = generate(&rq, 42, &r).unwrap();
    assert_eq!(a, b);
    assert_eq!(
        serde_json::to_vec(&a).unwrap(),
        serde_json::to_vec(&b).unwrap()
    );
    assert_ne!(a, generate(&rq, 43, &r).unwrap());
}

#[test]
fn perimeter_walled_and_edges_unique() {
    let m = gen(6, 5, 3, FeatureCounts::default());
    let mut keys = BTreeSet::new();
    for w in &m.walls {
        let e = w.edge.expect("grid edge");
        assert!(keys.insert((e.tile, e.side)), "duplicate {e:?}");
        assert!(w.height_m >= 0.125);
        if e.side == Dir::South {
            assert_eq!(e.tile.row, 0, "south only on the perimeter");
        }
        if e.side == Dir::West {
            assert_eq!(e.tile.col, 0, "west only on the perimeter");
        }
    }
    for t in &m.tiles {
        for d in Dir::ALL {
            if t.at.step(d, 6, 5).is_none() {
                assert!(m.has_wall(t.at, d));
            }
        }
    }
}

#[test]
fn perfect_maze_is_a_spanning_tree() {
    let m = gen(8, 6, 11, none());
    let internal = (7 * 6) + (8 * 5);
    let perimeter = 2 * (8 + 6);
    assert_eq!(m.walls.len(), perimeter + internal - (8 * 6 - 1));
    assert_ok(&m, &rules_2027_draft());
}

#[test]
fn loopiness_removes_walls() {
    let a = gen(8, 8, 5, none());
    let mut f = none();
    f.loopiness = 0.5;
    let b = gen(8, 8, 5, f);
    assert!(b.walls.len() < a.walls.len());
}

#[test]
fn start_on_perimeter_and_not_black() {
    for seed in 0..30 {
        let m = gen(5, 5, seed, FeatureCounts::default());
        let s = m.zones.start;
        let t = m.tile(s).unwrap();
        assert_eq!(t.kind, TileKind::Start);
        assert!(s.col == 0 || s.row == 0 || s.col == 4 || s.row == 4);
        assert!(!t.in_dz);
    }
}

#[test]
fn black_tiles_never_cut_the_maze() {
    let r = rules_2027_draft();
    let mut f = none();
    f.black = 8;
    for seed in 0..20 {
        let m = gen(6, 6, seed, f.clone());
        assert_ok(&m, &r);
        let ws = WallSet::from_graph(&m);
        let seen = reach(6, 6, &m.tiles, &ws, m.zones.start, false);
        for (t, s) in m.tiles.iter().zip(seen) {
            assert!(s || t.kind == TileKind::Black);
        }
    }
}

#[test]
fn silver_tiles_are_checkpoints() {
    let mut f = none();
    f.silver = 3;
    f.blue = 2;
    let m = gen(6, 6, 9, f);
    assert_eq!(m.zones.checkpoints.len(), 3);
    for c in &m.zones.checkpoints {
        assert_eq!(m.tile(*c).unwrap().kind, TileKind::Silver);
    }
    assert_eq!(
        m.tiles.iter().filter(|t| t.kind == TileKind::Blue).count(),
        2
    );
}

#[test]
fn dangerous_zone_is_sealed_and_optional() {
    let r = rules_2027_draft();
    let mut f = none();
    f.dangerous_zone = true;
    f.loopiness = 0.4;
    for seed in 0..30 {
        let m = gen(6, 6, seed, f.clone());
        let dz = m.zones.dangerous_zone.as_ref().expect("dz placed");
        assert_eq!(m.tile(dz.entrance).unwrap().kind, TileKind::Red);
        assert!(!m.tile(m.zones.start).unwrap().in_dz);
        let ws = WallSet::from_graph(&m);
        let seen = reach(6, 6, &m.tiles, &ws, m.zones.start, true);
        for (t, s) in m.tiles.iter().zip(seen) {
            assert!(s || t.in_dz || t.kind == TileKind::Black);
        }
        assert_ok(&m, &r);
    }
}

#[test]
fn dz_leak_is_detected() {
    let r = rules_2027_draft();
    let mut f = none();
    f.dangerous_zone = true;
    let mut m = gen(6, 6, 1, f);
    let dz = m.zones.dangerous_zone.clone().unwrap();
    let t = dz.tiles[0];
    m.walls.retain(|w| {
        let e = w.edge.unwrap();
        !(e.tile == t && e.side == Dir::North)
            && !(e.tile.row + 1 == t.row && e.tile.col == t.col && e.side == Dir::North)
    });
    assert!(!validate(&m, &r).ok);
}

fn find<F: Fn(&MazeGraph) -> bool>(w: u32, h: u32, f: &FeatureCounts, ok: F) -> MazeGraph {
    for seed in 0..200 {
        let m = gen(w, h, seed, f.clone());
        if ok(&m) {
            return m;
        }
    }
    panic!("no seed produced the feature");
}

#[test]
fn ramp_is_legal_and_heightfield_consistent() {
    let r = rules_2027_draft();
    let mut f = none();
    f.ramps = 1;
    f.loopiness = 0.2;
    let m = find(6, 6, &f, |m| m.tiles.iter().any(|t| !t.features.is_empty()));
    assert_ok(&m, &r);
    let ramp = m.tiles.iter().find(|t| !t.features.is_empty()).unwrap();
    let TileFeature::Ramp {
        up_dir,
        angle_deg,
        rise_m,
    } = ramp.features[0].clone()
    else {
        panic!("expected a ramp");
    };
    assert!(angle_deg <= r.field.ramp_max_deg);
    let plateau = m.tile(ramp.at.step(up_dir, 6, 6).unwrap()).unwrap();
    assert!((plateau.height_ref - rise_m).abs() < 1e-9);
    assert!(m.has_wall(ramp.at, Dir::ALL[(up_dir as usize + 1) % 4]));
}

#[test]
fn stairs_are_legal() {
    let r = rules_2027_draft();
    let mut f = none();
    f.stairs = 1;
    f.loopiness = 0.2;
    let m = find(6, 6, &f, |m| m.tiles.iter().any(|t| !t.features.is_empty()));
    assert_ok(&m, &r);
    let st = m.tiles.iter().find(|t| !t.features.is_empty()).unwrap();
    let TileFeature::Stairs {
        step_m, tread_m, ..
    } = st.features[0]
    else {
        panic!("expected stairs");
    };
    assert!(step_m <= r.field.stairs_step_max_m && tread_m >= r.field.stairs_tread_min_m);
}

#[test]
fn speed_bumps_obey_height_limits() {
    let r = rules_2027_draft();
    let mut f = none();
    f.speed_bumps = 5;
    let m = gen(6, 6, 4, f);
    let mut n = 0;
    for t in &m.tiles {
        for ft in &t.features {
            if let TileFeature::SpeedBump { height_m, .. } = ft {
                assert!(*height_m <= r.field.speed_bump_max_m);
                n += 1;
            }
        }
    }
    assert_eq!(n, 5);
    assert_ok(&m, &r);
}

#[test]
fn tall_bump_and_bump_on_ramp_are_rejected() {
    let r = rules_2027_draft();
    let mut f = none();
    f.speed_bumps = 1;
    let mut m = gen(5, 5, 2, f);
    for t in &mut m.tiles {
        for ft in &mut t.features {
            if let TileFeature::SpeedBump { height_m, .. } = ft {
                *height_m = 0.015;
            }
        }
    }
    assert!(!errors(&m, &r).is_empty());
}

#[test]
fn obstacles_touch_a_wall_and_are_tall_enough() {
    let r = rules_2027_draft();
    let mut f = none();
    f.obstacles = 3;
    let m = gen(6, 6, 8, f);
    assert_eq!(m.obstacles.len(), 3);
    for o in &m.obstacles {
        assert!(o.height_m >= r.field.obstacle_min_height_m);
        assert!(o.radius_m <= 0.04 + 1e-12);
    }
    assert_ok(&m, &r);
    let mut bad = m.clone();
    bad.obstacles[0].center = bad.obstacles[0].tile.center(r.field.tile_m);
    assert!(!validate(&bad, &r).ok);
}

#[test]
fn victims_sit_on_legal_walls() {
    let r = rules_2027_draft();
    let f = FeatureCounts {
        smiley_victims: 6,
        cognitive_victims: 6,
        decoys: 4,
        ..FeatureCounts::default()
    };
    let m = gen(8, 8, 21, f);
    assert_eq!(m.victims.len(), 16);
    for v in &m.victims {
        let t = m.tile(v.tile).unwrap();
        assert_eq!(t.kind, TileKind::Normal, "{}", ascii(&m));
        assert!(t.features.is_empty());
        assert!(m.has_wall(v.tile, v.wall_side));
        assert!((v.pos.z - (t.height_ref + 0.07)).abs() < 1e-9);
        assert_eq!(v.facing_yaw, v.wall_side.opposite().yaw());
    }
    assert_ok(&m, &r);
}

#[test]
fn cognitive_and_decoy_semantics() {
    let r = rules_2027_draft();
    let mut f = none();
    f.cognitive_victims = 6;
    f.decoys = 8;
    let m = gen(8, 8, 33, f);
    for v in &m.victims {
        match &v.kind {
            VictimKind::Cognitive { rings } => {
                let sum: i32 = rings.iter().map(|&c| r.ring_value(c)).sum();
                if (0..=2).contains(&sum) {
                    assert!(v.status.is_some() && v.tier.is_some());
                } else {
                    assert!(v.status.is_none() && v.tier.is_none());
                }
            }
            VictimKind::Decoy => assert!(v.status.is_none() && v.tier.is_none()),
            VictimKind::Smiley { .. } => unreachable!(),
        }
    }
    assert!(m.victims.iter().any(|v| v.tier.is_none()));
    assert_ok(&m, &r);
}

#[test]
fn victim_tiers_follow_wall_components() {
    let r = rules_2027_draft();
    let mut f = none();
    f.smiley_victims = 20;
    f.loopiness = 0.6;
    f.floating_victim_share = 1.0;
    let m = find(10, 10, &f, |m| {
        m.victims
            .iter()
            .any(|v| v.tier == Some(VictimTier::Floating))
    });
    assert_ok(&m, &r);
}

#[test]
fn classify_marks_floating_islands() {
    let mut m = gen(6, 6, 1, none());
    // Open every internal edge, then add a lone wall in the middle of the room.
    m.walls.retain(|w| {
        let e = w.edge.unwrap();
        (e.side == Dir::South && e.tile.row == 0)
            || (e.side == Dir::West && e.tile.col == 0)
            || (e.side == Dir::North && e.tile.row == 5)
            || (e.side == Dir::East && e.tile.col == 5)
    });
    let mut island = m.walls[0].clone();
    island.edge = Some(mm_core::WallEdge {
        tile: TileXY::new(2, 2),
        side: Dir::North,
    });
    m.walls.push(island);
    classify(&mut m);
    let class = |c, r| m.tile(TileXY::new(c, r)).unwrap().class;
    assert_eq!(class(2, 2), mm_core::TileClass::Floating);
    assert_eq!(class(2, 3), mm_core::TileClass::Floating);
    assert_eq!(class(0, 0), mm_core::TileClass::Linear);
    assert_eq!(class(2, 1), mm_core::TileClass::Floating);
}

#[test]
fn validator_flags_structural_errors() {
    let r = rules_2027_draft();
    let m = gen(5, 5, 6, FeatureCounts::default());
    assert_ok(&m, &r);

    let mut a = m.clone();
    a.tiles.pop();
    assert!(!validate(&a, &r).ok);

    let mut b = m.clone();
    b.walls
        .retain(|w| w.edge.unwrap().side != Dir::East || w.edge.unwrap().tile.col != 4);
    assert!(!validate(&b, &r).ok);

    let mut c = m.clone();
    c.walls.push(c.walls[0].clone());
    assert!(!validate(&c, &r).ok);

    let mut d = m.clone();
    d.walls[0].height_m = 0.05;
    assert!(!validate(&d, &r).ok);

    let mut e = m.clone();
    let i = idx(5, m.zones.start);
    e.tiles[i].kind = TileKind::Black;
    assert!(!validate(&e, &r).ok);
}

#[test]
fn warnings_are_reported_when_features_do_not_fit() {
    let r = rules_2027_draft();
    let f = FeatureCounts {
        black: 50,
        ..FeatureCounts::default()
    };
    let (m, warnings) = generate_with_report(&req(3, 3, f), 1, &r).unwrap();
    assert!(!warnings.is_empty());
    assert_ok(&m, &r);
}

#[test]
fn ascii_renders_every_row() {
    let m = gen(4, 3, 1, FeatureCounts::default());
    let s = ascii(&m);
    assert_eq!(s.lines().count(), 3 * 2 + 1);
    assert!(s.contains('S'));
}

fn arb_features() -> impl Strategy<Value = FeatureCounts> {
    (
        (0u32..4, 0u32..3, 0u32..3, 0u32..3, 0u32..2, 0u32..4),
        (0u32..3, 0u32..6, 0u32..6, 0u32..4),
        (any::<bool>(), 0.0f64..0.6, 0.0f64..1.0),
    )
        .prop_map(|(a, b, c)| FeatureCounts {
            black: a.0,
            silver: a.1,
            blue: a.2,
            ramps: a.3,
            stairs: a.4,
            speed_bumps: a.5,
            obstacles: b.0,
            smiley_victims: b.1,
            cognitive_victims: b.2,
            decoys: b.3,
            dangerous_zone: c.0,
            loopiness: c.1,
            floating_victim_share: c.2,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn generated_mazes_are_valid(
        w in 4u32..=12,
        h in 4u32..=12,
        seed in any::<u64>(),
        f in arb_features(),
    ) {
        let r = rules_2027_draft();
        let m = generate(&req(w, h, f.clone()), seed, &r).expect("generates");
        let v = validate(&m, &r);
        prop_assert!(v.ok, "{:#?}\n{}", v.issues, ascii(&m));
        prop_assert_eq!(m.tiles.len(), (w * h) as usize);
        prop_assert_eq!(v.reachable_tiles, v.enterable_tiles);

        let ws = WallSet::from_graph(&m);
        // Start: on the perimeter, not black, not in the DZ.
        let st = m.tile(m.zones.start).unwrap();
        prop_assert!(st.kind == TileKind::Start && !st.in_dz);
        // The DZ is optional: everything else is reachable around it.
        let seen = reach(w, h, &m.tiles, &ws, m.zones.start, true);
        for (t, s) in m.tiles.iter().zip(seen) {
            prop_assert!(s || t.in_dz || t.kind == TileKind::Black, "{}", ascii(&m));
        }
        // No duplicate wall edges.
        let mut keys = BTreeSet::new();
        for wl in &m.walls {
            let e = wl.edge.unwrap();
            prop_assert!(keys.insert(crate::grid::canonical(e.tile, e.side)));
        }
        // Victims on legal walls.
        for vc in &m.victims {
            let t = m.tile(vc.tile).unwrap();
            prop_assert!(t.kind == TileKind::Normal && t.features.is_empty());
            prop_assert!(m.has_wall(vc.tile, vc.wall_side));
            prop_assert!(!m.obstacles.iter().any(|o| o.tile == vc.tile));
        }
        // Determinism.
        prop_assert_eq!(generate(&req(w, h, f), seed, &r).unwrap(), m);
    }
}
