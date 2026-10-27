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
    let (s, _, v) = call(get("/api/runs/abc")).await;
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
