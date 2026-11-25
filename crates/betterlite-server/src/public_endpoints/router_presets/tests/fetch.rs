use std::time::Duration;

use axum::http::{Method, StatusCode};
use rstest::rstest;

use super::support::{
    PRESETS_URL, REMOTE_CATALOG, Script, app, bundled_catalog, get, moved, parsed, reply, settings,
};
use crate::public_endpoints::router_presets::AUTOROUTER_PRESETS_PATH;
use crate::public_endpoints::router_presets::catalog::decode_catalog;

#[tokio::test]
async fn fetches_from_the_configured_url_and_serves_the_catalog() {
    let script = Script::new(vec![reply(200, REMOTE_CATALOG)]);
    let router = app(&script, settings(PRESETS_URL, false));
    let (status, content_type, body) = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "application/json");
    assert_eq!(
        parsed(&body),
        decode_catalog(REMOTE_CATALOG.as_bytes()).unwrap()
    );
    let seen = script.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, Method::GET);
    assert_eq!(seen[0].url, PRESETS_URL);
}

#[tokio::test]
async fn applies_a_five_second_deadline_to_the_fetch() {
    let script = Script::new(vec![reply(200, REMOTE_CATALOG)]);
    let router = app(&script, settings(PRESETS_URL, false));
    let _response = get(router, AUTOROUTER_PRESETS_PATH).await;
    let seen = script.seen();
    assert_eq!(seen[0].deadline, Duration::from_secs(5));
}

#[tokio::test]
async fn serves_the_cached_catalog_without_a_second_fetch() {
    let script = Script::new(vec![reply(200, REMOTE_CATALOG), reply(200, "{}")]);
    let router = app(&script, settings(PRESETS_URL, false));
    let first = get(router.clone(), AUTOROUTER_PRESETS_PATH).await;
    let second = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(script.calls(), 1);
    assert_eq!(first.2, second.2);
    assert_eq!(
        parsed(&first.2),
        decode_catalog(REMOTE_CATALOG.as_bytes()).unwrap()
    );
}

#[rstest]
#[case::ok(200, true)]
#[case::created(201, true)]
#[case::redirect_without_a_location(300, false)]
#[case::not_modified(304, false)]
#[case::last_non_success(399, false)]
#[case::bad_request(400, false)]
#[case::not_found(404, false)]
#[case::unavailable(503, false)]
#[tokio::test]
async fn treats_only_2xx_answers_as_success(#[case] status: u16, #[case] remote: bool) {
    let script = Script::new(vec![reply(status, REMOTE_CATALOG)]);
    let router = app(&script, settings(PRESETS_URL, false));
    let (_, _, body) = get(router, AUTOROUTER_PRESETS_PATH).await;
    let expected = if remote {
        decode_catalog(REMOTE_CATALOG.as_bytes()).unwrap()
    } else {
        bundled_catalog()
    };
    assert_eq!(parsed(&body), expected);
}

#[tokio::test]
async fn takes_the_bundled_fallback_instead_of_following_a_redirect() {
    let script = Script::new(vec![
        moved(302, "http://mirror.test/catalog.json"),
        reply(200, REMOTE_CATALOG),
    ]);
    let router = app(&script, settings(PRESETS_URL, false));
    let (_, _, body) = get(router, AUTOROUTER_PRESETS_PATH).await;
    assert_eq!(parsed(&body), bundled_catalog());
    assert_eq!(script.calls(), 1);
}
