#![cfg(test)]

use axum::http::StatusCode;
use serde::Deserialize;

use crate::debug_routes::test_support;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CountsBody {
    num_items_in_user_api_key_cache: usize,
    num_items_in_llm_router_cache: usize,
    num_items_in_proxy_logging_obj_cache: usize,
}

#[tokio::test]
async fn counts_cache_and_ttl_entries_of_every_component() {
    let state = test_support::state_with(
        test_support::cache(test_support::dict(&["a", "b"]), test_support::ttl(&["a"])),
        test_support::cache(test_support::dict(&["c"]), test_support::ttl(&["c", "d"])),
        Some(test_support::cache(
            test_support::dict(&["r"]),
            test_support::ttl(&[]),
        )),
        test_support::cache(test_support::dict(&[]), test_support::ttl(&["p"])),
    );
    let router = test_support::protected(test_support::router(state, None));
    let (status, text) = test_support::call(
        router,
        "GET",
        "/memory-usage-in-mem-cache",
        Some(test_support::MASTER_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let counts: CountsBody = serde_json::from_str(&text).expect("a counts body");
    assert_eq!(counts.num_items_in_user_api_key_cache, 6);
    assert_eq!(counts.num_items_in_llm_router_cache, 1);
    assert_eq!(counts.num_items_in_proxy_logging_obj_cache, 1);
}

#[tokio::test]
async fn an_absent_router_cache_counts_zero_items() {
    let router = test_support::protected(test_support::router(test_support::state(), None));
    let (status, text) = test_support::call(
        router,
        "GET",
        "/memory-usage-in-mem-cache",
        Some(test_support::MASTER_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let counts: CountsBody = serde_json::from_str(&text).expect("a counts body");
    assert_eq!(counts.num_items_in_user_api_key_cache, 0);
    assert_eq!(counts.num_items_in_llm_router_cache, 0);
    assert_eq!(counts.num_items_in_proxy_logging_obj_cache, 0);
}

#[tokio::test]
async fn refuses_a_request_without_a_key() {
    let router = test_support::router(test_support::state(), None);
    let (status, text) =
        test_support::call(router, "GET", "/memory-usage-in-mem-cache", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(text.contains("\"type\":\"auth_error\""));
}
