#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use serde::Deserialize;

use crate::debug_routes::test_support;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemsBody {
    user_api_key_cache: BTreeMap<String, Json>,
    user_api_key_ttl: BTreeMap<String, f64>,
    user_key_object_cache: BTreeMap<String, Json>,
    user_key_object_ttl: BTreeMap<String, f64>,
    llm_router_cache: BTreeMap<String, Json>,
    llm_router_ttl: BTreeMap<String, f64>,
    proxy_logging_obj_cache: BTreeMap<String, Json>,
    proxy_logging_obj_ttl: BTreeMap<String, f64>,
}

#[tokio::test]
async fn returns_the_cache_and_ttl_dicts_of_every_component() {
    let state = test_support::state_with(
        test_support::cache(
            test_support::dict(&["key-1"]),
            test_support::ttl(&["key-1"]),
        ),
        test_support::cache(
            test_support::dict(&["key-object"]),
            test_support::ttl(&["key-object"]),
        ),
        Some(test_support::cache(
            test_support::dict(&["model"]),
            test_support::ttl(&["model"]),
        )),
        test_support::cache(
            test_support::dict(&["usage"]),
            test_support::ttl(&["usage"]),
        ),
    );
    let router = test_support::protected(test_support::router(
        state,
        &test_support::empty_environment(),
    ));
    let (status, text) = test_support::call(
        router,
        "GET",
        "/memory-usage-in-mem-cache-items",
        Some(test_support::MASTER_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body: ItemsBody = serde_json::from_str(&text).expect("an items body");
    assert_eq!(body.user_api_key_cache, test_support::dict(&["key-1"]));
    assert_eq!(body.user_api_key_ttl, test_support::ttl(&["key-1"]));
    assert_eq!(
        body.user_key_object_cache,
        test_support::dict(&["key-object"])
    );
    assert_eq!(body.user_key_object_ttl, test_support::ttl(&["key-object"]));
    assert_eq!(body.llm_router_cache, test_support::dict(&["model"]));
    assert_eq!(body.llm_router_ttl, test_support::ttl(&["model"]));
    assert_eq!(body.proxy_logging_obj_cache, test_support::dict(&["usage"]));
    assert_eq!(body.proxy_logging_obj_ttl, test_support::ttl(&["usage"]));
}

#[tokio::test]
async fn an_absent_router_cache_returns_empty_dicts() {
    let router = test_support::protected(test_support::router(
        test_support::state(),
        &test_support::empty_environment(),
    ));
    let (status, text) = test_support::call(
        router,
        "GET",
        "/memory-usage-in-mem-cache-items",
        Some(test_support::MASTER_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body: ItemsBody = serde_json::from_str(&text).expect("an items body");
    assert!(body.llm_router_cache.is_empty());
    assert!(body.llm_router_ttl.is_empty());
}

#[tokio::test]
async fn refuses_a_request_without_a_key() {
    let router = test_support::router(test_support::state(), &test_support::empty_environment());
    let (status, text) =
        test_support::call(router, "GET", "/memory-usage-in-mem-cache-items", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(text.contains("\"type\":\"auth_error\""));
}
