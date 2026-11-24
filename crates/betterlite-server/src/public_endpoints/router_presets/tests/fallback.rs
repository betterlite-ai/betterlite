use axum::http::StatusCode;
use rstest::rstest;
use serde::Deserialize;

use super::support::{
    Answer, PRESETS_URL, REMOTE_CATALOG, Script, app, app_with_bundled, bundled_catalog,
    capture_logs, get, parsed, reply, settings,
};
use crate::public_endpoints::router_presets::AUTOROUTER_PRESETS_PATH;

const UNKNOWN_TIER_CATALOG: &str = r#"{
  "broken": {
    "label": "Broken",
    "description": "A catalog with a tier the dashboard cannot apply.",
    "complexity_router_config": {
      "tiers": {"SIMPLE": [], "MEDIUM": [], "COMPLEX": [], "REASONING": [], "EXTRA": []}
    }
  }
}"#;

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

#[rstest]
#[case::server_error(503)]
#[case::not_found(404)]
#[tokio::test]
async fn falls_back_to_the_bundled_catalog_when_the_remote_answers_with_an_error(
    #[case] status: u16,
) {
    let script = Script::new(vec![reply(status, "")]);
    let router = app(&script, settings(PRESETS_URL, false));
    let (response_status, _, body) = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(response_status, StatusCode::OK);
    assert_eq!(parsed(&body), bundled_catalog());
    assert_eq!(script.calls(), 1);
}

#[tokio::test]
async fn logs_a_warning_when_the_fetch_fails() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);
    let script = Script::new(vec![reply(503, "")]);
    let router = app(&script, settings(PRESETS_URL, false));
    let (_, _, body) = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(parsed(&body), bundled_catalog());
    let contents = logs.contents();
    assert!(
        contents.contains("failed to fetch auto-router presets"),
        "{contents}"
    );
    assert!(contents.contains(PRESETS_URL), "{contents}");
}

#[tokio::test]
async fn falls_back_when_the_connection_fails() {
    let script = Script::new(vec![Answer::Closed]);
    let router = app(&script, settings(PRESETS_URL, false));
    let (_, _, body) = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(parsed(&body), bundled_catalog());
    assert_eq!(script.calls(), 1);
}

#[tokio::test]
async fn falls_back_when_the_remote_catalog_is_empty() {
    let script = Script::new(vec![reply(200, "{}")]);
    let router = app(&script, settings(PRESETS_URL, false));
    let (_, _, body) = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(parsed(&body), bundled_catalog());
}

#[tokio::test]
async fn falls_back_when_a_tier_is_unknown() {
    let script = Script::new(vec![reply(200, UNKNOWN_TIER_CATALOG)]);
    let router = app(&script, settings(PRESETS_URL, false));
    let (_, _, body) = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(parsed(&body), bundled_catalog());
}

#[tokio::test]
async fn falls_back_when_the_body_is_not_json() {
    let script = Script::new(vec![reply(200, "no catalog here")]);
    let router = app(&script, settings(PRESETS_URL, false));
    let (_, _, body) = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(parsed(&body), bundled_catalog());
}

#[tokio::test]
async fn caches_the_bundled_fallback() {
    let script = Script::new(vec![reply(503, ""), reply(200, REMOTE_CATALOG)]);
    let router = app(&script, settings(PRESETS_URL, false));
    let first = get(router.clone(), AUTOROUTER_PRESETS_PATH).await;
    let second = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(script.calls(), 1);
    assert_eq!(parsed(&first.2), bundled_catalog());
    assert_eq!(first.2, second.2);
}

#[tokio::test]
async fn falls_back_when_the_configured_url_is_invalid() {
    let script = Script::default();
    let router = app(&script, settings("not a url", false));
    let (_, _, body) = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(parsed(&body), bundled_catalog());
    assert_eq!(script.calls(), 0);
}

#[tokio::test]
async fn serves_the_bundled_catalog_in_local_only_mode_without_fetching() {
    let script = Script::new(vec![reply(200, REMOTE_CATALOG)]);
    let router = app(&script, settings(PRESETS_URL, true));
    let (status, _, body) = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parsed(&body), bundled_catalog());
    assert_eq!(script.calls(), 0);
}

#[tokio::test]
async fn bundled_catalog_decode_errors_propagate() {
    let script = Script::new(vec![reply(503, ""), reply(503, "")]);
    let router = app_with_bundled(&script, "{", false);
    let first = get(router.clone(), AUTOROUTER_PRESETS_PATH).await;
    let second = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(first.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(first.1, "application/json");
    let envelope: ErrorEnvelope = serde_json::from_str(&first.2).unwrap();
    assert_eq!(envelope.error.message, "Internal server error");
    assert_eq!(envelope.error.error_type, "internal_server_error");
    assert_eq!(envelope.error.param, None);
    assert_eq!(envelope.error.code, "500");
    assert_eq!(first.2, second.2);
    assert_eq!(script.calls(), 2);
}

#[tokio::test]
async fn local_only_mode_propagates_bundled_decode_errors_without_fetching() {
    let script = Script::default();
    let router = app_with_bundled(&script, "{", true);
    let (status, _, _) = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(script.calls(), 0);
}
