use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use tower::ServiceExt as _;

use crate::routes::tests::state_for;
use crate::state::ServerState;

const CONFIGURED_SOURCE: &str =
    "environment_variables:\n  BETTERLITE_LATEST_RELEASE_URL: not a url\n";

fn configured_state() -> Arc<ServerState> {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let config = loader
        .load(CONFIGURED_SOURCE)
        .expect("the test config should load");
    state_for(&config)
}

#[tokio::test]
async fn the_application_router_serves_the_configured_source() {
    let application = crate::release_info::routes(&configured_state());
    let request = Request::get("/get/latest_release_info")
        .body(Body::empty())
        .expect("the request should build");
    let response = application
        .oneshot(request)
        .await
        .expect("the router should answer");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
