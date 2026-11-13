#![cfg(test)]

use std::ops::Range;

use axum::http::StatusCode;
use betterlite_prompts::arguments::json::Object;
use rstest::rstest;

use crate::purview_graph_client::PurviewGuardrailBase;
use crate::purview_graph_client::error::PurviewGraphError;
use crate::purview_graph_client::process_content::{ContentRequest, process_content};
use crate::purview_graph_client::scope_cache::{CachedScope, compute_protection_scopes};
use crate::purview_graph_client::test_support::{
    MockReply, NOW_SECONDS, RecordedRequest, START_SECONDS, clock_at, guardrail, json_reply,
    standard_reply, start,
};
use crate::purview_graph_client::wire::Activity;

const COMPUTE_PATH: &str = "/protectionScopes/compute";

fn cached_scope(fetched_at_seconds: u64) -> CachedScope {
    CachedScope {
        etag: "old".to_owned(),
        response: Object::new(),
        fetched_at: clock_at(fetched_at_seconds),
    }
}

fn is_cached(base: &PurviewGuardrailBase, user_id: &str) -> bool {
    let caches = base.scope_and_token_cache_lock.lock().unwrap();
    caches.protection_scope_cache.contains(user_id)
}

#[tokio::test]
async fn scope_cache_serves_a_second_lookup_inside_the_ttl() {
    let mock = start(standard_reply).await;
    let base = guardrail(&mock.base);

    let (first_etag, first) = compute_protection_scopes(&base, "user-1").await.unwrap();
    let (second_etag, second) = compute_protection_scopes(&base, "user-1").await.unwrap();

    assert_eq!(first_etag, "\"etag-1\"");
    assert_eq!((second_etag, second), (first_etag, first));
    assert_eq!(mock.count(COMPUTE_PATH), 1);
}

#[rstest]
#[case::one_second_before_the_ttl(3599, 1)]
#[case::exactly_one_hour(3600, 2)]
#[tokio::test]
async fn scope_cache_refetches_after_one_hour(#[case] elapsed: u64, #[case] fetches: usize) {
    let mock = start(standard_reply).await;
    let base = guardrail(&mock.base);
    compute_protection_scopes(&base, "user-1").await.unwrap();

    NOW_SECONDS.set(START_SECONDS.saturating_add(elapsed));
    compute_protection_scopes(&base, "user-1").await.unwrap();

    assert_eq!(mock.count(COMPUTE_PATH), fetches);
}

#[tokio::test]
async fn scope_cache_posts_the_scope_request_for_the_encoded_user() {
    let mock = start(standard_reply).await;
    let base = guardrail(&mock.base);

    compute_protection_scopes(&base, "a b/c@contoso.com")
        .await
        .unwrap();

    let requests = mock.requests.lock().unwrap().clone();
    let request = requests.last().unwrap();
    assert_eq!(
        request.path,
        "/users/a%20b%2Fc%40contoso.com/dataSecurityAndGovernance/protectionScopes/compute"
    );
    assert_eq!(request.headers.get("authorization").unwrap(), "Bearer tok");
    assert_eq!(
        request.body,
        r#"{"activities":"uploadText,downloadText","locations":[{"@odata.type":"microsoft.graph.policyLocationApplication","value":"client-1"}]}"#
    );
}

#[rstest]
#[case::empty("")]
#[case::single_dot(".")]
#[case::double_dot("..")]
#[tokio::test]
async fn scope_cache_refuses_a_user_id_that_is_not_a_path_segment(#[case] user_id: &str) {
    let mock = start(standard_reply).await;
    let base = guardrail(&mock.base);

    let error = compute_protection_scopes(&base, user_id).await.unwrap_err();

    assert!(matches!(error, PurviewGraphError::UserId { .. }));
    assert_eq!(mock.count(COMPUTE_PATH), 0);
}

fn missing_etag(request: &RecordedRequest) -> MockReply {
    if request.path.ends_with("/token") {
        return standard_reply(request);
    }
    json_reply(r#"{"value":[]}"#)
}

fn upper_case_etag(request: &RecordedRequest) -> MockReply {
    if request.path.ends_with("/token") {
        return standard_reply(request);
    }
    MockReply {
        status: StatusCode::OK,
        headers: vec![("ETag", "W/1")],
        body: "{}".to_owned(),
    }
}

#[rstest]
#[case::absent(missing_etag, "")]
#[case::any_header_case(upper_case_etag, "W/1")]
#[tokio::test]
async fn scope_cache_reads_the_etag_header(
    #[case] responder: fn(&RecordedRequest) -> MockReply,
    #[case] expected: &str,
) {
    let mock = start(responder).await;
    let base = guardrail(&mock.base);

    let (etag, _) = compute_protection_scopes(&base, "user-1").await.unwrap();

    assert_eq!(etag, expected);
}

fn slow_scopes(request: &RecordedRequest) -> MockReply {
    if request.path.ends_with("/token") {
        return standard_reply(request);
    }
    NOW_SECONDS.set(START_SECONDS.saturating_add(3000));
    standard_reply(request)
}

#[tokio::test]
async fn scope_cache_counts_the_ttl_from_when_the_scopes_arrived() {
    let mock = start(slow_scopes).await;
    let base = guardrail(&mock.base);

    compute_protection_scopes(&base, "user-1").await.unwrap();

    let fetched_at = base
        .scope_and_token_cache_lock
        .lock()
        .unwrap()
        .protection_scope_cache
        .get("user-1")
        .unwrap()
        .fetched_at;
    assert_eq!(fetched_at, clock_at(START_SECONDS.saturating_add(3000)));
}

fn fill_scopes(base: &PurviewGuardrailBase, users: Range<u32>) {
    for index in users {
        base.scope_and_token_cache_lock
            .lock()
            .unwrap()
            .protection_scope_cache
            .put(format!("user-{index}"), cached_scope(START_SECONDS));
    }
}

#[tokio::test]
async fn scope_cache_holds_a_thousand_entries_and_evicts_the_least_recently_used() {
    let mock = start(standard_reply).await;
    let base = guardrail(&mock.base);
    fill_scopes(&base, 0..1000);
    compute_protection_scopes(&base, "user-0").await.unwrap();
    assert_eq!(mock.count(COMPUTE_PATH), 0);

    compute_protection_scopes(&base, "newcomer").await.unwrap();

    assert_eq!(mock.count(COMPUTE_PATH), 1);
    assert!(is_cached(&base, "user-0"));
    assert!(!is_cached(&base, "user-1"));
    assert!(is_cached(&base, "user-2"));
    assert_eq!(
        base.scope_and_token_cache_lock
            .lock()
            .unwrap()
            .protection_scope_cache
            .len(),
        1000
    );
}

#[tokio::test]
async fn scope_cache_treats_a_refreshed_entry_as_the_most_recently_used() {
    let mock = start(standard_reply).await;
    let base = guardrail(&mock.base);
    base.scope_and_token_cache_lock
        .lock()
        .unwrap()
        .protection_scope_cache
        .put(
            "user-0".to_owned(),
            cached_scope(START_SECONDS.saturating_sub(4000)),
        );
    fill_scopes(&base, 1..1000);
    compute_protection_scopes(&base, "user-0").await.unwrap();

    compute_protection_scopes(&base, "newcomer").await.unwrap();

    assert!(is_cached(&base, "user-0"));
    assert!(!is_cached(&base, "user-1"));
}

fn modified_policy(request: &RecordedRequest) -> MockReply {
    if request.path.ends_with("/processContent") {
        return json_reply(r#"{"protectionScopeState":"modified"}"#);
    }
    standard_reply(request)
}

fn unchanged_policy(request: &RecordedRequest) -> MockReply {
    if request.path.ends_with("/processContent") {
        return json_reply(r#"{"protectionScopeState":"notModified"}"#);
    }
    standard_reply(request)
}

fn scan_request(user_id: &str) -> ContentRequest<'_> {
    ContentRequest {
        user_id,
        text: "hello",
        activity: Activity::UploadText,
        etag: "\"etag-1\"",
        correlation_id: None,
    }
}

#[rstest]
#[case::modified(modified_policy, false)]
#[case::not_modified(unchanged_policy, true)]
#[tokio::test]
async fn scope_cache_drops_only_the_scanned_users_entry_when_the_policy_changed(
    #[case] responder: fn(&RecordedRequest) -> MockReply,
    #[case] scanned_user_stays: bool,
) {
    let mock = start(responder).await;
    let base = guardrail(&mock.base);
    compute_protection_scopes(&base, "user-1").await.unwrap();
    compute_protection_scopes(&base, "user-2").await.unwrap();

    process_content(&base, &scan_request("user-1"))
        .await
        .unwrap();

    assert_eq!(is_cached(&base, "user-1"), scanned_user_stays);
    assert!(is_cached(&base, "user-2"));
}

#[tokio::test]
async fn scope_cache_fetches_again_after_the_policy_changed() {
    let mock = start(modified_policy).await;
    let base = guardrail(&mock.base);
    compute_protection_scopes(&base, "user-1").await.unwrap();
    process_content(&base, &scan_request("user-1"))
        .await
        .unwrap();

    compute_protection_scopes(&base, "user-1").await.unwrap();

    assert_eq!(mock.count(COMPUTE_PATH), 2);
}
