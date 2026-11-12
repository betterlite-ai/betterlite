#![cfg(test)]

use crate::server::chat_setup::chat_executor;
use crate::state::ServerState;
use crate::state::shared::Shared;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tower::ServiceExt as _;

use super::router;

#[must_use]
pub(crate) fn empty_config() -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("")
    .unwrap()
}

#[must_use]
pub(crate) fn state_for(config: &Config) -> Arc<ServerState> {
    let executor = chat_executor(config, &BTreeMap::new(), StdRng::seed_from_u64(1)).unwrap();
    let shared = Arc::new(Shared);
    Arc::new(ServerState::new(config, Arc::new(executor), shared))
}

#[tokio::test]
async fn stub_routes_answer_not_found() {
    let config = empty_config();
    let application = router(
        state_for(&config),
        Arc::new(AtomicBool::new(false)),
        Router::new(),
        Router::new(),
    );
    let health = application
        .clone()
        .oneshot(
            Request::get("/health/liveness")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    let unserved = application
        .oneshot(
            Request::get("/v1/chat/completions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unserved.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(unserved.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&*body, br#"{"detail":"Not Found"}"#);
}
