use axum::http::StatusCode;

use super::support::{PRESETS_URL, REMOTE_CATALOG, Script, app, get, reply, settings};
use crate::public_endpoints::router_presets::AUTOROUTER_PRESETS_PATH;

#[tokio::test]
async fn concurrent_first_requests_share_one_fetch() {
    let script = Script::new(vec![reply(200, REMOTE_CATALOG)]);
    let router = app(&script, settings(PRESETS_URL, false));
    let (first, second) = tokio::join!(
        get(router.clone(), AUTOROUTER_PRESETS_PATH),
        get(router, AUTOROUTER_PRESETS_PATH)
    );
    assert_eq!(script.calls(), 1);
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(second.0, StatusCode::OK);
    assert_eq!(first.2, second.2);
}

#[tokio::test]
async fn concurrent_requests_after_the_first_answer_share_the_cache() {
    let script = Script::new(vec![reply(200, REMOTE_CATALOG)]);
    let router = app(&script, settings(PRESETS_URL, false));
    let warm = get(router.clone(), AUTOROUTER_PRESETS_PATH).await;
    let (first, second) = tokio::join!(
        get(router.clone(), AUTOROUTER_PRESETS_PATH),
        get(router, AUTOROUTER_PRESETS_PATH)
    );
    assert_eq!(script.calls(), 1);
    assert_eq!(warm.2, first.2);
    assert_eq!(first.2, second.2);
}
