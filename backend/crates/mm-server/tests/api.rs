use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use mm_core::RobotConfig;
use mm_server::{app, AppState};
use serde_json::Value;
use tower::ServiceExt;

async fn call(req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Value) {
    let res = app(AppState::for_tests()).oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, v)
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn health_shape() {
    let (s, _, v) = call(get("/api/health")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["ok"], true);
    assert_eq!(v["api_version"], mm_core::API_VERSION);
    assert_eq!(v["sim_capabilities"]["physics"], "kinematic");
    assert_eq!(
        v["sim_capabilities"]["sensors"].as_array().unwrap().len(),
        6
    );
    assert!(v["sim_capabilities"]["muse"].is_boolean());
}

#[tokio::test]
async fn spec_has_profile_and_presets() {
    let (s, _, v) = call(get("/api/spec")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["rules_profiles"].as_array().unwrap().len(), 1);
    assert_eq!(v["error_presets"].as_array().unwrap().len(), 3);
    assert_eq!(v["sensor_catalog"].as_array().unwrap().len(), 7);
    assert_eq!(v["reference_robot"]["tof"].as_array().unwrap().len(), 8);
    assert!(v["tunables"].as_array().unwrap().is_empty());
}

fn post_json(uri: &str, body: &Value) -> Request<Body> {
    Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn validate_flags_tall_robot() {
    let mut r = RobotConfig::reference();
    let (_, _, ok) = call(post_json(
        "/api/robots/validate",
        &serde_json::to_value(&r).unwrap(),
    ))
    .await;
    assert_eq!(ok["valid"], true);

    r.body.height_m = 0.30;
    let (s, _, v) = call(post_json(
        "/api/robots/validate",
        &serde_json::to_value(&r).unwrap(),
    ))
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["valid"], false);
    assert_eq!(v["issues"][0]["path"], "body.height_m");
}

#[tokio::test]
async fn validate_rejects_bad_body() {
    let (s, _, v) = call(post_json(
        "/api/robots/validate",
        &serde_json::json!({"nope": 1}),
    ))
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"]["code"], "bad_request");
}

#[tokio::test]
async fn unknown_route_is_json_404() {
    let (s, _, v) = call(get("/api/nope")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["code"], "not_found");
    assert!(v["error"]["details"].is_null());
}

#[tokio::test]
async fn planned_endpoints_are_501() {
    let (s, _, v) = call(post_json("/api/compare", &serde_json::json!({}))).await;
    assert_eq!(s, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(v["error"]["code"], "not_implemented");
}

fn preflight(origin: &str) -> Request<Body> {
    Request::options("/api/health")
        .header(header::ORIGIN, origin)
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn cors_allows_only_the_frontend() {
    let (_, h, _) = call(preflight("http://localhost:5173")).await;
    assert_eq!(
        h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
        "http://localhost:5173"
    );
    let (_, h, _) = call(preflight("http://evil.com")).await;
    assert!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
}

// ---- M2: robots and mazes ----

fn put_json(uri: &str, body: &Value) -> Request<Body> {
    Request::put(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// Shared-state client: every call hits the same in-memory database.
struct Client(axum::Router);

impl Client {
    fn new() -> Client {
        Client(app(AppState::for_tests()))
    }
    async fn call(&self, req: Request<Body>) -> (StatusCode, Value) {
        let res = self.0.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

fn gen_req(seed: Value, w: u32, h: u32) -> Value {
    serde_json::json!({ "seed": seed, "grid": { "w": w, "h": h } })
}

#[tokio::test]
async fn robot_round_trip() {
    let c = Client::new();
    let mut r = RobotConfig::reference();
    r.id = None;
    r.name = "Mk2".into();
    let (s, created) = c
        .call(post_json("/api/robots", &serde_json::to_value(&r).unwrap()))
        .await;
    assert_eq!(s, StatusCode::OK);
    let id = created["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("rob_") && id.len() == 16);

    let (s, got) = c.call(get(&format!("/api/robots/{id}"))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(got, created);

    let mut upd = got.clone();
    upd["name"] = "Mk3".into();
    let (s, v) = c.call(post_json("/api/robots", &upd)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["id"], id.as_str());
    let (_, got) = c.call(get(&format!("/api/robots/{id}"))).await;
    assert_eq!(got["name"], "Mk3");

    upd["id"] = "rob_nope".into();
    let (s, v) = c.call(post_json("/api/robots", &upd)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["code"], "not_found");
}

#[tokio::test]
async fn invalid_robot_is_422_with_issues() {
    let c = Client::new();
    let mut r = RobotConfig::reference();
    r.body.height_m = 0.30;
    let (s, v) = c
        .call(post_json("/api/robots", &serde_json::to_value(&r).unwrap()))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(v["error"]["code"], "validation_failed");
    assert_eq!(v["error"]["details"][0]["path"], "body.height_m");
}

#[tokio::test]
async fn reference_robot_is_seeded_and_listed() {
    let c = Client::new();
    let (s, v) = c.call(get("/api/robots/rob_reference")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["id"], "rob_reference");
    let (s, list) = c.call(get("/api/robots")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list[0]["id"], "rob_reference");
    assert!(list[0]["updated_at"].as_str().unwrap().ends_with('Z'));
    let (s, _) = c.call(get("/api/robots/rob_missing")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn generate_with_seed_is_deterministic() {
    let c = Client::new();
    let (s, a) = c
        .call(post_json("/api/mazes/generate", &gen_req(42.into(), 8, 8)))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, b) = c
        .call(post_json("/api/mazes/generate", &gen_req(42.into(), 8, 8)))
        .await;
    assert_eq!(a["maze"]["header"]["seed"], 42);
    let (mut ma, mut mb) = (a["maze"].clone(), b["maze"].clone());
    assert_ne!(ma["header"]["id"], mb["header"]["id"]);
    assert!(ma["header"]["id"].as_str().unwrap().starts_with("maz_"));
    ma["header"]["id"] = Value::Null;
    mb["header"]["id"] = Value::Null;
    assert_eq!(ma, mb);
    assert_eq!(a["ascii"], b["ascii"]);
    assert!(a["warnings"].is_array());
}

#[tokio::test]
async fn generate_without_seed_returns_small_seed() {
    let c = Client::new();
    let (s, v) = c
        .call(post_json(
            "/api/mazes/generate",
            &gen_req(Value::Null, 8, 8),
        ))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["maze"]["header"]["seed"].as_u64().unwrap() < (1u64 << 53));
}

#[tokio::test]
async fn generate_rejects_bad_input() {
    let c = Client::new();
    let (s, v) = c
        .call(post_json("/api/mazes/generate", &gen_req(1.into(), 1, 1)))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"]["code"], "bad_request");
    let mut req = gen_req(1.into(), 8, 8);
    req["rules"] = "nope".into();
    let (s, _) = c.call(post_json("/api/mazes/generate", &req)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn maze_store_get_list_put_validate() {
    let c = Client::new();
    let (s, v) = c.call(get("/api/mazes/maz_missing")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["code"], "not_found");

    let (_, g) = c
        .call(post_json("/api/mazes/generate", &gen_req(7.into(), 6, 5)))
        .await;
    let id = g["maze"]["header"]["id"].as_str().unwrap().to_string();

    let (s, stored) = c.call(get(&format!("/api/mazes/{id}"))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(stored, g["maze"]);

    let (_, list) = c.call(get("/api/mazes")).await;
    assert_eq!(list[0]["id"], id.as_str());
    assert_eq!(list[0]["seed"], 7);
    assert_eq!(list[0]["w"], 6);
    assert_eq!(list[0]["h"], 5);

    let (s, v) = c.call(post_json("/api/mazes/validate", &stored)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["ok"], true);

    // Edit: rename. Must come back with a validation block and persist.
    let mut edited = stored.clone();
    edited["header"]["name"] = "Edited".into();
    let (s, v) = c.call(put_json(&format!("/api/mazes/{id}"), &edited)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["maze"]["header"]["name"], "Edited");
    assert!(v["validation"]["ok"].is_boolean());
    let (_, again) = c.call(get(&format!("/api/mazes/{id}"))).await;
    assert_eq!(again["header"]["name"], "Edited");

    let (s, _) = c.call(put_json("/api/mazes/maz_missing", &edited)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let mut broken = stored;
    broken["tiles"] = serde_json::json!([]);
    let (s, _) = c.call(put_json(&format!("/api/mazes/{id}"), &broken)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// ---- M3: controllers, runs, WebSocket ----

use std::time::Duration;

use serde_json::json;

impl Client {
    async fn raw(&self, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let res = self.0.clone().oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, headers, bytes.to_vec())
    }
}

async fn make_maze(c: &Client) -> String {
    let (_, g) = c
        .call(post_json("/api/mazes/generate", &gen_req(5.into(), 5, 5)))
        .await;
    g["maze"]["header"]["id"].as_str().unwrap().to_string()
}

fn run_body(maze_id: &str, seed: u64, speed: f64, max_s: f64) -> Value {
    json!({
        "robot_id": "rob_reference", "maze_id": maze_id, "controller_id": "ctl_wall_follower",
        "rng": {"mode": "fixed", "seed": seed}, "error_profile_id": "realistic",
        "speed": speed, "max_duration_s": max_s, "lop_enabled": true, "params": {}
    })
}

async fn wait_ended(c: &Client, id: &str) -> Value {
    for _ in 0..3000 {
        let (_, v) = c.call(get(&format!("/api/runs/{id}"))).await;
        if matches!(v["state"].as_str(), Some("finished" | "stopped" | "failed")) {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("run {id} did not end");
}

#[tokio::test]
async fn controller_validate_good_and_bad() {
    let c = Client::new();
    let (s, ok) = c
        .call(post_json(
            "/api/controllers/validate",
            &json!({"code": mm_controller::WALL_FOLLOWER_RHAI}),
        ))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ok["ok"], true);
    assert!(!ok["tunables"].as_array().unwrap().is_empty());
    let (s, bad) = c
        .call(post_json(
            "/api/controllers/validate",
            &json!({"code": "fn step(obs {"}),
        ))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(bad["ok"], false);
    assert!(!bad["errors"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn controller_versions_and_builtins() {
    let c = Client::new();
    let (_, list) = c.call(get("/api/controllers")).await;
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["ctl_wall_follower", "ctl_victim_check", "ctl_idle"]);

    let code1 = "fn step(obs) { #{ wheel_v: [0.0, 0.0, 0.0, 0.0] } }";
    let code2 = "fn step(obs) { #{ wheel_v: [0.1, 0.1, 0.1, 0.1] } }";
    let (s, a) = c
        .call(post_json(
            "/api/controllers",
            &json!({"name": "mine", "code": code1}),
        ))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(a["version"], 1);
    assert_eq!(a["validation"]["ok"], true);
    let id = a["id"].as_str().unwrap().to_string();
    let (_, b) = c
        .call(post_json(
            "/api/controllers",
            &json!({"id": id, "name": "mine v2", "code": code2}),
        ))
        .await;
    assert_eq!(b["version"], 2);
    let (_, latest) = c.call(get(&format!("/api/controllers/{id}"))).await;
    assert_eq!(latest["version"], 2);
    assert_eq!(latest["code"], code2);
    let (_, v1) = c
        .call(get(&format!("/api/controllers/{id}?version=1")))
        .await;
    assert_eq!(v1["code"], code1);
    let (_, vs) = c
        .call(get(&format!("/api/controllers/{id}/versions")))
        .await;
    assert_eq!(vs.as_array().unwrap().len(), 2);
    let (_, list) = c.call(get("/api/controllers")).await;
    assert_eq!(list.as_array().unwrap().len(), 4);

    let (s, e) = c
        .call(post_json(
            "/api/controllers",
            &json!({"name": "x", "code": "fn step("}),
        ))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(e["error"]["code"], "validation_failed");
    let (s, _) = c
        .call(post_json(
            "/api/controllers",
            &json!({"id": "ctl_nope", "name": "x", "code": code1}),
        ))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = c.call(get("/api/controllers/ctl_nope")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, p) = c.call(get("/api/controllers/conversion-prompt")).await;
    assert!(p["prompt"].as_str().unwrap().len() > 100);
}

fn normalise(bytes: &[u8]) -> Vec<u8> {
    let mut r = mm_sim::decode_msgpack(bytes).unwrap();
    r.header.run_id = String::new();
    r.header.created_at = None;
    mm_sim::encode_msgpack(&r).unwrap()
}

#[tokio::test]
async fn fixed_seed_run_is_reproducible() {
    let c = Client::new();
    let maze = make_maze(&c).await;
    let body = run_body(&maze, 42, 0.0, 6.0);
    let (s, a) = c.call(post_json("/api/runs", &body)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(a["seed"], 42);
    assert_eq!(a["state"], "queued");
    let (_, b) = c.call(post_json("/api/runs", &body)).await;
    let (ida, idb) = (a["id"].as_str().unwrap(), b["id"].as_str().unwrap());
    assert!(ida.starts_with("run_") && ida != idb);

    let done = wait_ended(&c, ida).await;
    wait_ended(&c, idb).await;
    assert_eq!(done["state"], "finished");
    assert_eq!(done["end_reason"], "time_up");
    assert!(done["metrics"]["score"]["total"].is_number());
    assert!(done["metrics"]["duration_s"].as_f64().unwrap() > 5.0);

    let accept = |id: &str| {
        Request::get(format!("/api/runs/{id}/replay"))
            .header(header::ACCEPT, "application/msgpack")
            .body(Body::empty())
            .unwrap()
    };
    let (s, h, ma) = c.raw(accept(ida)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h[header::CONTENT_TYPE], "application/msgpack");
    let (_, _, mb) = c.raw(accept(idb)).await;
    assert_eq!(normalise(&ma), normalise(&mb));
    let replay = mm_sim::decode_msgpack(&ma).unwrap();
    assert_eq!(replay.header.seed, 42);
    assert!(replay.header.created_at.is_some());

    let (s, rj) = c.call(get(&format!("/api/runs/{ida}/replay"))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(rj["header"]["run_id"], ida);
    assert_eq!(rj["frames"].as_array().unwrap().len(), replay.frames.len());
    let (_, m) = c.call(get(&format!("/api/runs/{ida}/metrics"))).await;
    assert_eq!(m, done["metrics"]);

    let (_, list) = c.call(get("/api/runs?limit=1")).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], idb);
    assert_eq!(list[0]["seed"], 42);
    assert!(list[0]["score_total"].is_number());
}

#[tokio::test]
async fn run_creation_errors() {
    let c = Client::new();
    let maze = make_maze(&c).await;
    let mut b = run_body(&maze, 1, 0.0, 5.0);
    b["robot_id"] = "rob_nope".into();
    let (s, v) = c.call(post_json("/api/runs", &b)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["code"], "not_found");
    for (field, val) in [
        ("maze_id", "maz_nope"),
        ("controller_id", "ctl_nope"),
        ("error_profile_id", "nope"),
    ] {
        let mut b = run_body(&maze, 1, 0.0, 5.0);
        b[field] = val.into();
        let (s, _) = c.call(post_json("/api/runs", &b)).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{field}");
    }
    let mut b = run_body(&maze, 1, 0.0, 5.0);
    b["rng"] = json!({"mode": "fixed", "seed": null});
    let (s, _) = c.call(post_json("/api/runs", &b)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = c.call(get("/api/runs/run_nope")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = c
        .call(post_json("/api/runs/run_nope/stop", &json!({})))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // trng draws a fresh seed below 2^53.
    let mut b = run_body(&maze, 1, 0.0, 2.0);
    b["rng"] = json!({"mode": "trng", "seed": null});
    let (s, v) = c.call(post_json("/api/runs", &b)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["seed"].as_u64().unwrap() < (1u64 << 53));
    wait_ended(&c, v["id"].as_str().unwrap()).await;
}

#[tokio::test]
async fn pause_resume_and_stop_a_long_run() {
    let c = Client::new();
    let maze = make_maze(&c).await;
    let (_, r) = c
        .call(post_json("/api/runs", &run_body(&maze, 3, 1.0, 600.0)))
        .await;
    let id = r["id"].as_str().unwrap();
    let act = |what: &str| post_json(&format!("/api/runs/{id}/{what}"), &json!({}));

    let (s, v) = c.call(act("resume")).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let (s, v) = c.call(act("pause")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["state"], "paused");
    let (s, _) = c.call(act("pause")).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = c.call(act("resume")).await;
    assert_eq!(s, StatusCode::OK);

    let (s, v) = c.call(act("stop")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["state"], "stopped");
    let done = wait_ended(&c, id).await;
    assert_eq!(done["end_reason"], "stopped");
    assert!(done["metrics"]["duration_s"].as_f64().unwrap() < 60.0);
    let (s, _, _) = c.raw(get(&format!("/api/runs/{id}/replay"))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = c.call(act("stop")).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn batch_runs_in_parallel() {
    let c = Client::new();
    let maze = make_maze(&c).await;
    let (s, v) = c
        .call(post_json(
            "/api/runs/batch",
            &json!({"base": run_body(&maze, 0, 5.0, 3.0), "seeds": [1, 2, 3]}),
        ))
        .await;
    assert_eq!(s, StatusCode::OK);
    let runs = v["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 3);
    assert!(runs.iter().all(|r| r["state"] == "finished"));
    assert_eq!(runs[1]["seed"], 2);
    assert_eq!(v["summary"]["count"], 3);
    assert_eq!(v["summary"]["scored"], 3);
    let (min, max) = (
        v["summary"]["min_score"].as_i64().unwrap(),
        v["summary"]["max_score"].as_i64().unwrap(),
    );
    assert!(min <= max && v["summary"]["mean_score"].as_f64().unwrap() >= min as f64);
    let (s, _) = c
        .call(post_json(
            "/api/runs/batch",
            &json!({"base": run_body(&maze, 0, 0.0, 3.0), "seeds": []}),
        ))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn websocket_streams_hello_frames_events_end() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;

    let state = AppState::for_tests();
    let c = Client(app(state.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app(state)).await.unwrap() });

    let maze = make_maze(&c).await;
    let (_, r) = c
        .call(post_json("/api/runs", &run_body(&maze, 9, 10.0, 8.0)))
        .await;
    let id = r["id"].as_str().unwrap().to_string();

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/runs/{id}"))
        .await
        .unwrap();
    let mut types = Vec::new();
    let mut events = 0;
    let mut end = Value::Null;
    while let Some(m) = tokio::time::timeout(Duration::from_secs(30), ws.next())
        .await
        .unwrap()
    {
        let Message::Text(t) = m.unwrap() else {
            continue;
        };
        let v: Value = serde_json::from_str(&t).unwrap();
        let ty = v["type"].as_str().unwrap().to_string();
        if ty == "event" {
            assert!(v["event"]["type"].is_string() && v["t"].is_number());
            events += 1;
        }
        if ty == "end" {
            end = v.clone();
        }
        if types.is_empty() {
            assert_eq!(ty, "hello");
            assert_eq!(v["maze"]["header"]["id"], maze.as_str());
            assert_eq!(v["run"]["id"], id.as_str());
        }
        types.push(ty);
    }
    assert!(
        types.iter().filter(|t| *t == "frame").count() > 3,
        "{types:?}"
    );
    assert_eq!(types.last().unwrap(), "end");
    assert_eq!(end["reason"], "time_up");
    assert!(end["metrics"]["score"]["total"].is_number());

    // Every event of the replay reached the client.
    let (_, _, blob) = c
        .raw(
            Request::get(format!("/api/runs/{id}/replay?format=msgpack"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(events, mm_sim::decode_msgpack(&blob).unwrap().events.len());

    // A finished run: hello, end, close.
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/runs/{id}"))
        .await
        .unwrap();
    let mut seen = Vec::new();
    while let Some(Ok(m)) = ws.next().await {
        if let Message::Text(t) = m {
            let v: Value = serde_json::from_str(&t).unwrap();
            seen.push(v["type"].as_str().unwrap().to_string());
        }
    }
    assert_eq!(seen, ["hello", "end"]);
}
