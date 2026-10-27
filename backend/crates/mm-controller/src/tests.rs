use mm_core::{Controller, Observations, RobotConfig, RobotInfo};

use crate::*;

fn info() -> RobotInfo {
    RobotInfo {
        robot: RobotConfig::reference(),
        tile_m: 0.28,
        controller_dt_s: 0.01,
        params: Default::default(),
    }
}

fn obs(t: f64) -> Observations {
    stub_observations(&RobotConfig::reference(), t)
}

fn make(code: &str) -> RhaiController {
    let mut c = RhaiController::new(code, Limits::default()).expect("compiles");
    c.init(&info()).expect("init");
    c
}

#[test]
fn tunables_parse_good_lines() {
    let code = "// @tunable wall_dist 0.10 0.05 0.20 0.01 \"Target side-wall distance (m)\"\n  // @tunable k 1 0 5 0.5 \"Gain\"\nfn step(obs) {}";
    let t = parse_tunables(code).unwrap();
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].path, "controller.wall_dist");
    assert_eq!(t[0].description, "Target side-wall distance (m)");
    assert_eq!(
        (t[0].default, t[0].min, t[0].max, t[0].max_step),
        (0.10, 0.05, 0.20, 0.01)
    );
}

#[test]
fn tunables_reject_bad_lines() {
    let code = [
        "// @tunable a 5 0 1 0.1 \"default above max\"",
        "// @tunable b 0.5 0 1 0.1 no quotes",
        "// @tunable c x 0 1 0.1 \"not a number\"",
        "// @tunable d 0.5 0 1",
        "// @tunable e 0.5 0 1 0 \"zero step\"",
        "// @tunable f 0.5 0 1 0.1 \"ok\"",
        "// @tunable f 0.5 0 1 0.1 \"duplicate\"",
    ]
    .join("\n");
    let errs = parse_tunables(&code).unwrap_err();
    assert_eq!(errs.len(), 6, "{errs:?}");
    assert_eq!(errs[0].line, Some(1));
    assert!(errs.iter().any(|e| e.message.contains("duplicate")));
}

#[test]
fn fuel_exhaustion_is_a_clean_error() {
    let limits = Limits {
        max_operations: 10_000,
        ..Limits::default()
    };
    let mut c = RhaiController::new("fn step(obs) { loop {} }", limits).unwrap();
    c.init(&info()).unwrap();
    let e = c.step(&obs(0.0)).unwrap_err();
    assert!(e.contains("fuel") || e.contains("operations"), "{e}");
    // The controller is still usable afterwards.
    assert!(c.step(&obs(0.01)).is_err());
}

#[test]
fn this_state_persists_across_steps() {
    let code = "fn init(info) { this.n = 0; }\nfn step(obs) { this.n += 1; #{ wheel_v: [this.n.to_float(), 0.0, 0.0, 0.0] } }";
    let mut c = make(code);
    for i in 1..=5 {
        let cmd = c.step(&obs(0.0)).unwrap();
        assert_eq!(cmd.wheel_v[0], f64::from(i));
    }
}

#[test]
fn params_default_and_override() {
    let code =
        "// @tunable k 2.0 0.0 5.0 0.5 \"gain\"\nfn step(obs) { #{ left: PARAMS.k, right: 1 } }";
    let mut c = make(code);
    assert_eq!(c.step(&obs(0.0)).unwrap().wheel_v, [2.0, 1.0, 2.0, 1.0]);
    let mut i = info();
    i.params.insert("controller.k".into(), 3.0);
    c.init(&i).unwrap();
    assert_eq!(c.step(&obs(0.0)).unwrap().wheel_v[0], 3.0);
}

#[test]
fn bad_return_values_error() {
    let cases = [
        (
            "fn step(obs) { #{ wheel_v: [0.0, 0.0, 0.0, 0.0], bogus: 1, other: 2 } }",
            "bogus, other",
        ),
        ("fn step(obs) { #{ wheel_v: [0.0, 0.0] } }", "4 numbers"),
        ("fn step(obs) { #{ left: 1.0 / 0.0 } }", "finite"),
        ("fn step(obs) { 42 }", "object map"),
        ("fn step(obs) { #{ report: \"dead\" } }", "report"),
        ("fn step(obs) { #{ deploy_kits: -1 } }", "deploy_kits"),
    ];
    for (code, needle) in cases {
        let mut c = make(code);
        let e = c.step(&obs(0.0)).unwrap_err();
        assert!(e.contains(needle), "{code}: {e}");
    }
}

#[test]
fn unit_return_means_defaults_and_report_parses() {
    let mut c = make("fn step(obs) { }");
    assert_eq!(c.step(&obs(0.0)).unwrap(), Default::default());
    let mut c = make("fn step(obs) { #{ report: \"harmed\", deploy_kits: 2, note: \"hi\" } }");
    let cmd = c.step(&obs(0.0)).unwrap();
    assert_eq!(cmd.deploy_kits, 2);
    assert!(cmd.report.is_some());
    assert_eq!(cmd.note.as_deref(), Some("hi"));
}

#[test]
fn syntax_error_has_line_and_col() {
    let e = RhaiController::new("fn step(obs) {\n    let x = ;\n}", Limits::default())
        .err()
        .expect("must fail");
    assert_eq!(e.errors[0].line, Some(2), "{e}");
    assert!(e.errors[0].col.is_some());
}

#[test]
fn runtime_error_has_line() {
    let mut c = make("fn step(obs) {\n\n    let x = obs.nope.deeper;\n    x.boom()\n}");
    let e = c.step(&obs(0.0)).unwrap_err();
    assert!(e.contains("line"), "{e}");
}

#[test]
fn missing_step_is_rejected() {
    assert!(RhaiController::new("fn init(info) {}", Limits::default()).is_err());
}

#[test]
fn print_goes_to_the_log_not_stdout() {
    let mut c = make("fn step(obs) { print(\"hello\"); debug(1 + 1); }");
    c.step(&obs(0.0)).unwrap();
    let log = c.take_log();
    assert_eq!(log.len(), 2, "{log:?}");
    assert_eq!(log[0], "hello");
}

#[test]
fn bundled_scripts_validate() {
    let robot = RobotConfig::reference();
    for (name, code) in [
        ("wall_follower", WALL_FOLLOWER_RHAI),
        ("victim_check", VICTIM_CHECK_RHAI),
    ] {
        let out = validate(code, &robot);
        assert!(out.ok, "{name}: {:?}", out.errors);
        assert_eq!(out.dry_run_trace.len(), 50);
        assert!(!out.tunables.is_empty());
        let json = serde_json::to_string(&out).unwrap();
        assert!(json.contains("dry_run_trace"));
    }
}

#[test]
fn validate_reports_errors_and_skips_dry_run() {
    let robot = RobotConfig::reference();
    let out = validate("fn step(obs) { loop {} }", &robot);
    assert!(!out.ok);
    assert_eq!(out.errors.len(), 1);
    let out = validate("fn step(obs { }", &robot);
    assert!(!out.ok && out.errors[0].line == Some(1));
}

#[test]
fn conversion_prompt_embeds_both_scripts() {
    assert!(CONVERSION_PROMPT.contains(WALL_FOLLOWER_RHAI.trim()));
    assert!(CONVERSION_PROMPT.contains(VICTIM_CHECK_RHAI.trim()));
}

#[test]
fn bundled_scripts_move_the_robot() {
    let robot = RobotConfig::reference();
    let out = validate(WALL_FOLLOWER_RHAI, &robot);
    let last = out.dry_run_trace.last().unwrap();
    assert!(last.commands.wheel_v.iter().any(|v| v.abs() > 0.0));
}

#[test]
fn builtin_wall_follower_runs_100_steps() {
    let mut w = WallFollower::new();
    w.init(&info()).unwrap();
    let mut moved = false;
    for i in 0..100 {
        let c = w.step(&obs(f64::from(i) * 0.01)).unwrap();
        assert!(c.wheel_v.iter().all(|v| v.is_finite()));
        moved |= c.wheel_v.iter().any(|v| *v != 0.0);
    }
    assert!(moved);
    let mut idle = Idle;
    assert_eq!(idle.step(&obs(0.0)).unwrap(), Default::default());
}

#[test]
fn same_script_same_obs_is_deterministic() {
    let run = || {
        let mut c = make(VICTIM_CHECK_RHAI);
        (0..30)
            .map(|i| c.step(&obs(f64::from(i) * 0.01)).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}
