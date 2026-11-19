use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use tower::ServiceExt as _;

use crate::release_info::routes;
use crate::routes::router;
use crate::routes::tests::state_for;
use crate::state::ServerState;

const CONFIGURED_SOURCE: &str = "\
general_settings:
  master_key: oracle-master
environment_variables:
  BETTERLITE_LATEST_RELEASE_URL: not a url
";

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
    let application = routes(&configured_state());
    let request = Request::get("/get/latest_release_info")
        .body(Body::empty())
        .expect("the request should build");
    let response = application
        .oneshot(request)
        .await
        .expect("the router should answer");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn the_application_router_mounts_the_route_behind_auth() {
    let application = router(
        configured_state(),
        Arc::new(AtomicBool::new(false)),
        Router::new(),
        Router::new(),
    );
    let keyless = application
        .clone()
        .oneshot(
            Request::get("/get/latest_release_info")
                .body(Body::empty())
                .expect("the request should build"),
        )
        .await
        .expect("the router should answer");
    assert_eq!(keyless.status(), StatusCode::UNAUTHORIZED);
    let keyed = application
        .oneshot(
            Request::get("/get/latest_release_info")
                .header("authorization", "Bearer oracle-master")
                .body(Body::empty())
                .expect("the request should build"),
        )
        .await
        .expect("the router should answer");
    assert_eq!(keyed.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
