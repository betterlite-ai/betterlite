use axum::http::{Method, StatusCode};
use rstest::rstest;

use super::support::{PRESETS_URL, REMOTE_CATALOG, Script, app, get, reply, send, settings};
use crate::public_endpoints::router_presets::{
    AUTOROUTER_PRESETS_PATH, FUSE_PRESETS_PATH, SCORER_DEFAULTS_PATH,
};

#[rstest]
#[case::autorouter(AUTOROUTER_PRESETS_PATH)]
#[case::scorer_defaults(SCORER_DEFAULTS_PATH)]
#[case::fuse_presets(FUSE_PRESETS_PATH)]
#[tokio::test]
async fn serves_each_public_route_as_json(#[case] path: &str) {
    let script = Script::new(vec![reply(200, REMOTE_CATALOG)]);
    let router = app(&script, settings(PRESETS_URL, false));
    let (status, content_type, _body) = get(router, path).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "application/json");
}

#[rstest]
#[case::autorouter(AUTOROUTER_PRESETS_PATH)]
#[case::scorer_defaults(SCORER_DEFAULTS_PATH)]
#[case::fuse_presets(FUSE_PRESETS_PATH)]
#[tokio::test]
async fn rejects_non_get_methods(#[case] path: &str) {
    let router = app(&Script::default(), settings(PRESETS_URL, true));
    let (status, _, _body) = send(router, Method::POST, path).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn other_paths_are_not_these_routes() {
    let router = app(&Script::default(), settings(PRESETS_URL, true));
    let (status, _, _body) = get(router, "/public/router_presets").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
