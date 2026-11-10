#![cfg(test)]

mod support;

use std::sync::Arc;

use rstest::rstest;
use serde_json::json;

use self::support::{CallMethod, RecordingCalls, TestResponse, qdrant_embedding};
use super::error::SemanticEmbeddingError;
use super::redis::redis_embedding;
use super::{EmbeddingCachePolicy, EmbeddingMode, EmbeddingRoute, MetadataMap};
use crate::semantic_embedding_inputs::build_router_embedding_metadata;

const MODEL: &str = support::MODEL;

fn caller_metadata() -> MetadataMap {
    let mut metadata = MetadataMap::new();
    metadata.insert("user".to_owned(), json!("a"));
    metadata
}

#[rstest]
#[case::listed_deployment(true, Some(MODEL), EmbeddingRoute::Router)]
#[case::unlisted_deployment(true, Some("other-embedding-model"), EmbeddingRoute::Direct)]
#[case::no_router(false, Some(MODEL), EmbeddingRoute::Direct)]
#[case::no_model_list(true, None, EmbeddingRoute::Direct)]
#[tokio::test]
async fn embedding_route_follows_the_injected_router_state(
    #[case] router_present: bool,
    #[case] listed: Option<&str>,
    #[case] expected: EmbeddingRoute,
) {
    let calls = RecordingCalls::succeeding(vec![vec![0.5_f64]]);
    let entries = listed.map(support::model_list);
    let service = support::service(
        support::settings(None, None),
        router_present.then(|| support::router(Some(1000))),
        entries,
        &calls,
        Arc::new(support::selector()),
    );
    redis_embedding(&service, "hello", None, EmbeddingMode::Sync)
        .await
        .unwrap();
    let recorded = calls.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].request.route, expected);
}

#[rstest]
#[case::sync_embedding_uses_the_sync_entry_point(EmbeddingMode::Sync, CallMethod::Sync)]
#[case::async_embedding_uses_the_async_entry_point(EmbeddingMode::Async, CallMethod::Async)]
#[tokio::test]
async fn embedding_method_matches_the_cache_path(
    #[case] mode: EmbeddingMode,
    #[case] expected: CallMethod,
) {
    let calls = RecordingCalls::succeeding(vec![vec![0.5_f64]]);
    let service = support::service(
        support::settings(None, None),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    redis_embedding(&service, "hello", None, mode)
        .await
        .unwrap();
    qdrant_embedding(&service, "hello", None, mode)
        .await
        .unwrap();
    let recorded = calls.recorded();
    assert_eq!(recorded.len(), 2);
    assert!(recorded.iter().all(|call| call.method == expected));
}

#[tokio::test]
async fn embedding_router_calls_carry_a_flagged_metadata_copy() {
    let calls = RecordingCalls::succeeding(vec![vec![0.5_f64]]);
    let service = support::service(
        support::settings(None, None),
        Some(support::router(Some(10))),
        Some(support::model_list(MODEL)),
        &calls,
        Arc::new(support::selector()),
    );
    let metadata = caller_metadata();
    redis_embedding(&service, "hello", Some(&metadata), EmbeddingMode::Sync)
        .await
        .unwrap();
    let request = &calls.recorded()[0].request;
    let expected = build_router_embedding_metadata(Some(&metadata));
    assert_eq!(request.metadata.as_ref(), Some(&expected));
    assert_eq!(metadata, caller_metadata());
}

#[tokio::test]
async fn embedding_direct_calls_forward_no_metadata() {
    let calls = RecordingCalls::succeeding(vec![vec![0.5_f64]]);
    let service = support::service(
        support::settings(None, None),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    let metadata = caller_metadata();
    redis_embedding(&service, "hello", Some(&metadata), EmbeddingMode::Sync)
        .await
        .unwrap();
    let request = &calls.recorded()[0].request;
    assert_eq!(request.metadata, None);
}

#[rstest]
#[case::configured_timeout(Some(2.5_f64), 2.5_f64)]
#[case::zero_timeout(Some(0.0_f64), 0.0_f64)]
#[case::default_timeout(None, support::DEFAULT_TIMEOUT)]
#[tokio::test]
async fn embedding_requests_carry_the_cache_flags_retries_and_timeout(
    #[case] configured: Option<f64>,
    #[case] expected: f64,
) {
    let calls = RecordingCalls::succeeding(vec![vec![0.5_f64]]);
    let service = support::service(
        support::settings(None, configured),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    qdrant_embedding(&service, "hello", None, EmbeddingMode::Sync)
        .await
        .unwrap();
    let request = &calls.recorded()[0].request;
    assert_eq!(
        request.cache,
        EmbeddingCachePolicy {
            no_store: true,
            no_cache: true,
        }
    );
    assert_eq!(request.num_retries, 0);
    assert_eq!(request.timeout.to_bits(), expected.to_bits());
}

#[derive(Clone, Copy, Debug)]
enum DeploymentLimit {
    Absent,
    Unlimited,
    Limit(i64),
}

#[rstest]
#[case::configured_limit(Some(1), DeploymentLimit::Absent, None, "hello")]
#[case::configured_limit_above_the_prompt(Some(5), DeploymentLimit::Absent, None, "hello world")]
#[case::no_limit(None, DeploymentLimit::Absent, None, "hello world")]
#[case::deployment_limit(None, DeploymentLimit::Limit(1), Some(MODEL), "hello")]
#[case::configured_beats_the_deployment(
    Some(5),
    DeploymentLimit::Limit(1),
    Some(MODEL),
    "hello world"
)]
#[case::deployment_without_a_limit(None, DeploymentLimit::Unlimited, Some(MODEL), "hello world")]
#[case::unlisted_deployment_has_no_limit(
    None,
    DeploymentLimit::Limit(1),
    Some("other-embedding-model"),
    "hello world"
)]
#[tokio::test]
async fn embedding_input_is_truncated_by_the_resolved_max_tokens(
    #[case] configured: Option<i64>,
    #[case] deployment: DeploymentLimit,
    #[case] listed: Option<&str>,
    #[case] expected: &str,
) {
    let calls = RecordingCalls::succeeding(vec![vec![0.5_f64]]);
    let entries = listed.map(support::model_list);
    let router = match deployment {
        DeploymentLimit::Absent => None,
        DeploymentLimit::Unlimited => Some(support::router(None)),
        DeploymentLimit::Limit(tokens) => Some(support::router(Some(tokens))),
    };
    let service = support::service(
        support::settings(configured, None),
        router,
        entries,
        &calls,
        Arc::new(support::selector()),
    );
    redis_embedding(&service, "hello world", None, EmbeddingMode::Sync)
        .await
        .unwrap();
    assert_eq!(calls.recorded()[0].request.input, expected);
}

#[tokio::test]
async fn embedding_without_a_max_input_token_skips_the_tokenizer() {
    let calls = RecordingCalls::succeeding(vec![vec![0.5_f64]]);
    let service = support::service(
        support::settings(None, None),
        None,
        None,
        &calls,
        Arc::new(support::failing_selector()),
    );
    redis_embedding(&service, "hello world", None, EmbeddingMode::Sync)
        .await
        .unwrap();
    assert_eq!(calls.recorded()[0].request.input, "hello world");
}

#[tokio::test]
async fn embedding_preparation_errors_propagate_unwrapped() {
    let calls = RecordingCalls::succeeding(vec![vec![0.5_f64]]);
    let service = support::service(
        support::settings(Some(1), None),
        None,
        None,
        &calls,
        Arc::new(support::failing_selector()),
    );
    let async_error = redis_embedding(&service, "hello world", None, EmbeddingMode::Async)
        .await
        .unwrap_err();
    assert!(matches!(async_error, SemanticEmbeddingError::Input(_)));
    assert_eq!(async_error.status_code(), None);
    let qdrant_error = qdrant_embedding(&service, "hello world", None, EmbeddingMode::Async)
        .await
        .unwrap_err();
    assert!(matches!(qdrant_error, SemanticEmbeddingError::Input(_)));
    let sync_error = redis_embedding(&service, "hello world", None, EmbeddingMode::Sync)
        .await
        .unwrap_err();
    assert!(matches!(sync_error, SemanticEmbeddingError::Input(_)));
    assert_eq!(calls.recorded().len(), 0);
}

#[tokio::test]
async fn redis_embedding_returns_only_the_first_vector() {
    let calls = RecordingCalls::succeeding(vec![vec![0.25_f64, 0.5_f64], vec![0.75_f64, 1.0_f64]]);
    let service = support::service(
        support::settings(None, None),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    assert_eq!(
        redis_embedding(&service, "hello", None, EmbeddingMode::Sync)
            .await
            .unwrap(),
        vec![0.25_f64, 0.5_f64]
    );
    assert_eq!(
        redis_embedding(&service, "hello", None, EmbeddingMode::Async)
            .await
            .unwrap(),
        vec![0.25_f64, 0.5_f64]
    );
}

#[tokio::test]
async fn qdrant_embedding_returns_the_whole_reply() {
    let calls = RecordingCalls::succeeding(vec![vec![0.25_f64, 0.5_f64], vec![0.75_f64, 1.0_f64]]);
    let service = support::service(
        support::settings(None, None),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    let whole = TestResponse {
        vectors: vec![vec![0.25_f64, 0.5_f64], vec![0.75_f64, 1.0_f64]],
    };
    assert_eq!(
        qdrant_embedding(&service, "hello", None, EmbeddingMode::Sync)
            .await
            .unwrap(),
        whole
    );
    assert_eq!(
        qdrant_embedding(&service, "hello", None, EmbeddingMode::Async)
            .await
            .unwrap(),
        whole
    );
}

#[tokio::test]
async fn embedding_sync_paths_propagate_call_failures() {
    let calls = RecordingCalls::failing(500);
    let service = support::service(
        support::settings(None, None),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    let redis_error = redis_embedding(&service, "hello", None, EmbeddingMode::Sync)
        .await
        .unwrap_err();
    assert!(matches!(redis_error, SemanticEmbeddingError::Call(_)));
    assert_eq!(redis_error.status_code(), Some(500));
    let qdrant_error = qdrant_embedding(&service, "hello", None, EmbeddingMode::Sync)
        .await
        .unwrap_err();
    assert!(matches!(qdrant_error, SemanticEmbeddingError::Call(_)));
    assert_eq!(qdrant_error.status_code(), Some(500));
}

#[tokio::test]
async fn redis_async_embedding_wraps_call_failures() {
    let calls = RecordingCalls::failing(500);
    let service = support::service(
        support::settings(None, None),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    let error = redis_embedding(&service, "hello", None, EmbeddingMode::Async)
        .await
        .unwrap_err();
    assert!(matches!(error, SemanticEmbeddingError::Failed { .. }));
    assert_eq!(error.status_code(), None);
}

#[tokio::test]
async fn qdrant_async_embedding_propagates_call_failures() {
    let calls = RecordingCalls::failing(500);
    let service = support::service(
        support::settings(None, None),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    let error = qdrant_embedding(&service, "hello", None, EmbeddingMode::Async)
        .await
        .unwrap_err();
    assert!(matches!(error, SemanticEmbeddingError::Call(_)));
    assert_eq!(error.status_code(), Some(500));
}

#[tokio::test]
async fn redis_embedding_reports_a_reply_without_a_vector() {
    let calls = RecordingCalls::succeeding(Vec::new());
    let service = support::service(
        support::settings(None, None),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    let sync_error = redis_embedding(&service, "hello", None, EmbeddingMode::Sync)
        .await
        .unwrap_err();
    assert!(matches!(sync_error, SemanticEmbeddingError::Reply(_)));
    assert_eq!(sync_error.status_code(), None);
    let async_error = redis_embedding(&service, "hello", None, EmbeddingMode::Async)
        .await
        .unwrap_err();
    assert!(matches!(async_error, SemanticEmbeddingError::Failed { .. }));
}

#[tokio::test]
async fn qdrant_embedding_returns_a_reply_without_a_vector() {
    let calls = RecordingCalls::succeeding(Vec::new());
    let service = support::service(
        support::settings(None, None),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    let empty = TestResponse {
        vectors: Vec::new(),
    };
    assert_eq!(
        qdrant_embedding(&service, "hello", None, EmbeddingMode::Sync)
            .await
            .unwrap(),
        empty
    );
    assert_eq!(
        qdrant_embedding(&service, "hello", None, EmbeddingMode::Async)
            .await
            .unwrap(),
        empty
    );
}

#[tokio::test(start_paused = true)]
async fn redis_async_embedding_wraps_the_wait_for_timeout() {
    let calls = RecordingCalls::hanging();
    let service = support::service(
        support::settings(None, Some(1.5_f64)),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    let error = redis_embedding(&service, "hello", None, EmbeddingMode::Async)
        .await
        .unwrap_err();
    assert!(matches!(error, SemanticEmbeddingError::Failed { .. }));
    assert_eq!(error.status_code(), None);
}

#[tokio::test(start_paused = true)]
async fn qdrant_async_embedding_propagates_the_wait_for_timeout() {
    let calls = RecordingCalls::hanging();
    let service = support::service(
        support::settings(None, Some(1.5_f64)),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    let error = qdrant_embedding(&service, "hello", None, EmbeddingMode::Async)
        .await
        .unwrap_err();
    let SemanticEmbeddingError::Timeout { seconds } = error else {
        panic!("expected a timeout");
    };
    assert_eq!(seconds.to_bits(), 1.5_f64.to_bits());
}

#[tokio::test(start_paused = true)]
async fn non_positive_embedding_timeout_times_out_at_once() {
    let calls = RecordingCalls::succeeding(vec![vec![0.5_f64]]);
    let service = support::service(
        support::settings(None, Some(-1.0_f64)),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    let qdrant_error = qdrant_embedding(&service, "hello", None, EmbeddingMode::Async)
        .await
        .unwrap_err();
    assert!(matches!(
        qdrant_error,
        SemanticEmbeddingError::Timeout { .. }
    ));
    let redis_error = redis_embedding(&service, "hello", None, EmbeddingMode::Async)
        .await
        .unwrap_err();
    assert!(matches!(redis_error, SemanticEmbeddingError::Failed { .. }));
    assert_eq!(calls.recorded().len(), 0);
}

const ZERO_TIMEOUT: f64 = 0.0;
const NEGATIVE_TIMEOUT: f64 = -1.0;
const NAN_TIMEOUT: f64 = f64::NAN;
const INFINITE_TIMEOUT: f64 = f64::INFINITY;
const HUGE_TIMEOUT: f64 = f64::MAX;

#[rstest]
#[case::zero(ZERO_TIMEOUT)]
#[case::negative(NEGATIVE_TIMEOUT)]
#[case::nan(NAN_TIMEOUT)]
#[tokio::test(start_paused = true)]
async fn unusable_embedding_timeout_skips_the_call(#[case] timeout: f64) {
    let calls = RecordingCalls::succeeding(vec![vec![0.5_f64]]);
    let service = support::service(
        support::settings(None, Some(timeout)),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    let error = qdrant_embedding(&service, "hello", None, EmbeddingMode::Async)
        .await
        .unwrap_err();
    assert!(matches!(error, SemanticEmbeddingError::Timeout { .. }));
    assert_eq!(calls.recorded().len(), 0);
}

#[rstest]
#[case::infinite(INFINITE_TIMEOUT)]
#[case::huge(HUGE_TIMEOUT)]
#[tokio::test(start_paused = true)]
async fn unbounded_embedding_timeout_runs_the_call(#[case] timeout: f64) {
    let calls = RecordingCalls::succeeding(vec![vec![0.5_f64]]);
    let service = support::service(
        support::settings(None, Some(timeout)),
        None,
        None,
        &calls,
        Arc::new(support::selector()),
    );
    let reply = qdrant_embedding(&service, "hello", None, EmbeddingMode::Async)
        .await
        .unwrap();
    assert_eq!(reply.vectors, vec![vec![0.5_f64]]);
    let recorded = calls.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].request.timeout.to_bits(), timeout.to_bits());
}

#[test]
fn embedding_reply_without_a_vector_is_empty() {
    let response = TestResponse {
        vectors: Vec::new(),
    };
    assert!(super::EmbeddedReply::first_embedding(&response).is_none());
}
