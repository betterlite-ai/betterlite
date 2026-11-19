use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde::Deserialize;
use tokio::sync::Mutex;
use tower::ServiceExt as _;

use super::support::{Answer, RELEASE_URL, Script, fake_clock};
use crate::release_info::records::LatestReleaseInfo;
use crate::release_info::route;
use crate::release_info::service::ReleaseService;

const RELEASE_PATH: &str = "/get/latest_release_info";
const MAX_BODY: usize = 64 * 1024;
const RELEASE_RECORD: &str = r#"{"tag_name":"v2.4.6","html_url":"https://example.test/release","body":"* feat: a\n* fix: b\n"}"#;

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
}

fn app(script: Script, source: &str) -> Router {
    let (clock, _now) = fake_clock(1_000.0);
    let service = Arc::new(ReleaseService {
        client: script,
        source: source.to_owned(),
        clock,
        cache: Mutex::new(None),
        fetch_lock: Mutex::new(()),
    });
    route::router(service)
}

fn working_app() -> Router {
    let script = Script::new(vec![Answer::Reply {
        status: 200,
        body: RELEASE_RECORD.as_bytes().to_vec(),
    }]);
    app(script, RELEASE_URL)
}

async fn call(app: Router, path: &str) -> (StatusCode, String, String) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .expect("the request should build");
    let response = app
        .oneshot(request)
        .await
        .expect("the router should answer");
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = to_bytes(response.into_body(), MAX_BODY)
        .await
        .expect("the body should be readable");
    let text = String::from_utf8(body.to_vec()).expect("the body should be UTF-8");
    (status, content_type, text)
}

#[tokio::test]
async fn answers_the_release_as_json() {
    let (status, content_type, text) = call(working_app(), RELEASE_PATH).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "application/json");
    let info: LatestReleaseInfo =
        serde_json::from_str(&text).expect("the body should be release info");
    assert_eq!(info.version, "2.4.6");
    assert_eq!(info.new_features, 1);
    assert_eq!(info.bug_fixes, 1);
    assert_eq!(info.other_updates, 0);
    assert_eq!(info.release_url, "https://example.test/release");
}

#[tokio::test]
async fn answers_null_when_the_source_is_unavailable() {
    let script = Script::new(vec![Answer::Reply {
        status: 500,
        body: Vec::new(),
    }]);
    let (status, content_type, text) = call(app(script, RELEASE_URL), RELEASE_PATH).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "application/json");
    assert_eq!(text, "null");
}

#[tokio::test]
async fn answers_an_error_when_the_source_cannot_be_read() {
    let (status, content_type, text) =
        call(app(Script::default(), "not a url"), RELEASE_PATH).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(content_type, "application/json");
    let envelope: ErrorEnvelope =
        serde_json::from_str(&text).expect("the body should be an error envelope");
    assert_eq!(envelope.error.message, "Internal server error");
    assert_eq!(envelope.error.error_type, "internal_server_error");
    assert_eq!(envelope.error.param, None);
    assert_eq!(envelope.error.code, "500");
}

#[tokio::test]
async fn other_paths_are_not_this_route() {
    let (status, _content_type, _text) = call(working_app(), "/get/other_info").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
