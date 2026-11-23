#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::body::Body;
use axum::http::header::AUTHORIZATION;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use tower::ServiceExt as _;

use super::{mount, mount_public, start, wrap};
use crate::routes::router;
use crate::routes::tests::{empty_config, state_for};

#[must_use]
fn master_key_config() -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  master_key: sk-master\n")
    .unwrap()
}

async fn status_through(router: Router) -> StatusCode {
    router
        .oneshot(Request::get("/through").body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn aggregator_keeps_area_routes_behind_auth() {
    let application = router(
        state_for(&master_key_config()),
        Arc::new(AtomicBool::new(false)),
        Router::new(),
        Router::new(),
    );
    let refused = application
        .clone()
        .oneshot(
            Request::get("/global/activity/cache_hits")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    let allowed = application
        .oneshot(
            Request::get("/global/activity/cache_hits")
                .header(AUTHORIZATION, "Bearer sk-master")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(allowed.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn aggregator_stubs_pass_router_through() {
    let state = state_for(&empty_config());
    let route = Router::new().route("/through", get(|| async { "ok" }));
    assert_eq!(
        status_through(mount(route.clone(), &state)).await,
        StatusCode::OK
    );
    assert_eq!(
        status_through(mount_public(route.clone(), &state)).await,
        StatusCode::OK
    );
    assert_eq!(status_through(wrap(route, &state)).await, StatusCode::OK);
}

#[tokio::test]
async fn aggregator_start_completes() {
    start(&state_for(&empty_config())).await;
}
