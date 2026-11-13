use betterlite_models::decisions::{HiddenParams, Json};
use chrono::{DateTime, TimeDelta, Utc};
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{embedding, extra, start, usage};
use crate::embedding_merge::combine::merge_with_api_result;
use crate::embedding_merge::error::MergeError;
use crate::embedding_merge::response::{
    ApiEmbeddingResponse, CachedEmbeddingResponse, Embedding, MergedEmbeddingResponse,
};
use crate::embedding_merge::usage::Usage;

fn cached(data: Vec<Option<Embedding>>) -> CachedEmbeddingResponse {
    CachedEmbeddingResponse {
        model: Some("text-embedding-3-small".to_owned()),
        data,
        usage: Some(usage([1, 0, 1], None)),
        hidden_params: HiddenParams::from([("cache_hit".to_owned(), Json::Bool(true))]),
        response_headers: None,
    }
}

fn fresh(data: Vec<Embedding>) -> ApiEmbeddingResponse {
    ApiEmbeddingResponse {
        data,
        usage: Some(usage([2, 0, 2], None)),
    }
}

fn merge_over(
    cached: CachedEmbeddingResponse,
    fresh: &ApiEmbeddingResponse,
    micros: i64,
) -> MergedEmbeddingResponse {
    let end = start()
        .checked_add_signed(TimeDelta::microseconds(micros))
        .expect("the end is in range");
    merge_with_api_result(Some(cached), fresh, start(), end)
        .expect("the responses merge")
        .expect("a cached response gives a merged one")
}

fn merge(cached: CachedEmbeddingResponse, fresh: &ApiEmbeddingResponse) -> MergedEmbeddingResponse {
    merge_over(cached, fresh, 0)
}

#[test]
fn leaves_the_api_response_alone_without_a_cached_one() {
    let merged = merge_with_api_result(None, &fresh(vec![embedding("x", 0)]), start(), start())
        .expect("nothing to merge");
    assert_eq!(merged, None);
}

#[test]
fn fills_the_empty_positions_in_order_with_the_position_as_index() {
    let cached = cached(vec![
        Some(embedding("a", 0)),
        None,
        Some(embedding("c", 2)),
        None,
    ]);
    let merged = merge(cached, &fresh(vec![embedding("x", 0), embedding("y", 1)]));
    assert_eq!(
        merged.data,
        [
            embedding("a", 0),
            embedding("x", 1),
            embedding("c", 2),
            embedding("y", 3)
        ]
    );
}

#[test]
fn keeps_every_cached_item_when_no_position_is_empty() {
    let cached = cached(vec![Some(embedding("a", 0)), Some(embedding("b", 1))]);
    let merged = merge(cached, &fresh(vec![embedding("x", 0), embedding("y", 1)]));
    assert_eq!(merged.data, [embedding("a", 0), embedding("b", 1)]);
}

#[test]
fn ignores_the_fresh_embeddings_beyond_the_empty_positions() {
    let cached = cached(vec![None, Some(embedding("b", 1))]);
    let merged = merge(cached, &fresh(vec![embedding("x", 0), embedding("y", 1)]));
    assert_eq!(merged.data, [embedding("x", 0), embedding("b", 1)]);
}

#[test]
fn fails_when_the_fresh_embeddings_run_out() {
    let cached = cached(vec![None, Some(embedding("b", 1)), None]);
    let error = merge_with_api_result(
        Some(cached),
        &fresh(vec![embedding("x", 0)]),
        start(),
        start(),
    )
    .expect_err("one position has no fresh embedding");
    assert!(matches!(error, MergeError::MissingFresh { position: 2 }));
}

#[rstest]
#[case::both_present(Some(usage([1, 0, 1], None)), Some(usage([2, 0, 2], None)), usage([3, 0, 3], None))]
#[case::fresh_absent(Some(usage([1, 0, 1], None)), None, usage([1, 0, 1], None))]
#[case::cached_absent(None, Some(usage([2, 0, 2], None)), Usage::default())]
#[case::both_absent(None, None, Usage::default())]
fn takes_the_usage_from_the_sides_that_have_one(
    #[case] cached_usage: Option<Usage>,
    #[case] fresh_usage: Option<Usage>,
    #[case] expected: Usage,
) {
    let mut cached = cached(vec![None]);
    cached.usage = cached_usage;
    let mut fresh = fresh(vec![embedding("x", 0)]);
    fresh.usage = fresh_usage;
    assert_eq!(merge(cached, &fresh).usage, expected);
}

#[test]
fn merges_the_prompt_token_details_of_both_usages() {
    let mut cached = cached(vec![None]);
    cached.usage = Some(usage([1, 0, 1], Some(r#"{"cached_tokens": 1}"#)));
    let mut fresh = fresh(vec![embedding("x", 0)]);
    fresh.usage = Some(usage([2, 0, 2], Some(r#"{"cached_tokens": 2}"#)));
    let merged = merge(cached, &fresh);
    assert_eq!(
        merged.usage,
        usage([3, 0, 3], Some(r#"{"cached_tokens": 3}"#))
    );
}

#[test]
fn passes_a_usage_overflow_on() {
    let mut cached = cached(vec![None]);
    cached.usage = Some(usage([i64::MAX, 0, 0], None));
    let error = merge_with_api_result(
        Some(cached),
        &fresh(vec![embedding("x", 0)]),
        start(),
        start(),
    )
    .expect_err("the prompt tokens overflow");
    assert!(matches!(
        error,
        MergeError::Overflow {
            field: "prompt_tokens"
        }
    ));
}

#[test]
fn takes_the_model_from_the_cached_response() {
    let merged = merge(cached(vec![None]), &fresh(vec![embedding("x", 0)]));
    assert_eq!(merged.model.as_deref(), Some("text-embedding-3-small"));
    let mut unnamed = cached(vec![None]);
    unnamed.model = None;
    assert_eq!(merge(unnamed, &fresh(vec![embedding("x", 0)])).model, None);
}

#[test]
fn copies_the_hidden_params_and_marks_the_cache_hit() {
    let mut cached = cached(vec![None]);
    cached.hidden_params = extra(r#"{"custom_llm_provider": "openai", "cache_hit": false}"#);
    let merged = merge(cached, &fresh(vec![embedding("x", 0)]));
    assert_eq!(
        merged.hidden_params,
        extra(r#"{"custom_llm_provider": "openai", "cache_hit": true}"#)
    );
}

#[test]
fn adds_the_cache_hit_mark_when_the_cached_params_lack_it() {
    let mut cached = cached(vec![None]);
    cached.hidden_params = HiddenParams::new();
    let merged = merge(cached, &fresh(vec![embedding("x", 0)]));
    assert_eq!(merged.hidden_params, extra(r#"{"cache_hit": true}"#));
}

#[rstest]
#[case::kept(Some(r#"{"x-request-id": "r1"}"#), Some(r#"{"x-request-id": "r1"}"#))]
#[case::empty_is_dropped(Some("{}"), None)]
#[case::absent(None, None)]
fn copies_the_non_empty_response_headers(
    #[case] headers: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut cached = cached(vec![None]);
    cached.response_headers = headers.map(extra);
    let merged = merge(cached, &fresh(vec![embedding("x", 0)]));
    assert_eq!(merged.response_headers, expected.map(extra));
}

#[rstest]
#[case::instant(0, Decimal::new(0, 0))]
#[case::microsecond_precision(1, Decimal::new(1, 3))]
#[case::milliseconds(1_500, Decimal::new(15, 1))]
#[case::seconds(2_500_000, Decimal::new(2500, 0))]
#[case::end_before_start(-1_500, Decimal::new(-15, 1))]
fn measures_the_response_time_in_milliseconds(#[case] micros: i64, #[case] expected: Decimal) {
    let merged = merge_over(cached(vec![None]), &fresh(vec![embedding("x", 0)]), micros);
    assert_eq!(merged.response_ms, expected);
}

#[test]
fn rejects_a_time_range_that_does_not_fit_microseconds() {
    let error = merge_with_api_result(
        Some(cached(vec![None])),
        &fresh(vec![embedding("x", 0)]),
        DateTime::<Utc>::MIN_UTC,
        DateTime::<Utc>::MAX_UTC,
    )
    .expect_err("the range is too long");
    assert!(matches!(error, MergeError::ResponseTime));
}
