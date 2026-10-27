use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use mm_core::RobotConfig;
use mm_server::{app, AppState};
use serde_json::Value;
use tower::ServiceExt;

async fn call(req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Value) {
    let res = app(AppState::default()).oneshot(req).await.unwrap();
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
