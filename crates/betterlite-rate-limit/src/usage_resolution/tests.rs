#![cfg(test)]

use super::{
    ResponseObject, ResponseUsage, TokenRateLimitType, Usage, aggregate_only_total_tokens,
    get_rate_limit_type, get_total_tokens_from_usage, reconcile_io_usage,
    recovered_partial_usage_tokens, resolve_rerank_token_usage, response_usage,
};

fn usage() -> Usage {
    Usage {
        prompt_tokens: 100,
        completion_tokens: 20,
        total_tokens: 120,
        cached_tokens: Some(30),
        ..Usage::default()
    }
}

#[test]
fn rate_limit_type_behaviors() {
    assert_eq!(
        get_rate_limit_type(TokenRateLimitType::Total),
        TokenRateLimitType::Total
    );
    assert_eq!(
        TokenRateLimitType::from_config(Some("input")),
        TokenRateLimitType::Input
    );
    assert_eq!(
        TokenRateLimitType::from_config(Some("invalid")),
        TokenRateLimitType::Total
    );
}

#[test]
fn usage_filtering_behaviors() {
    let value = usage();
    assert_eq!(
        get_total_tokens_from_usage(
            Some(ResponseUsage::Usage(&value)),
            TokenRateLimitType::Output
        ),
        20
    );
    assert_eq!(
        get_total_tokens_from_usage(
            Some(ResponseUsage::Usage(&value)),
            TokenRateLimitType::Input
        ),
        70
    );
    assert_eq!(
        get_total_tokens_from_usage(
            Some(ResponseUsage::Usage(&value)),
            TokenRateLimitType::Total
        ),
        90
    );
    let aggregate = Usage {
        total_tokens: 120,
        ..Usage::default()
    };
    assert_eq!(
        aggregate_only_total_tokens(Some(ResponseUsage::Usage(&aggregate))),
        120
    );
    assert_eq!(
        aggregate_only_total_tokens(Some(ResponseUsage::Usage(&value))),
        0
    );
    assert!(matches!(
        response_usage(&ResponseObject::Mapping(value)),
        Some(ResponseUsage::Usage(_))
    ));
}

#[test]
fn usage_filtering_boundaries() {
    assert_eq!(
        get_total_tokens_from_usage(None, TokenRateLimitType::Total),
        0
    );
    assert_eq!(response_usage(&ResponseObject::WithUsage(None)), None);
}

#[test]
fn rerank_usage_behaviors() {
    assert_eq!(
        resolve_rerank_token_usage(Some((8, 2)), None),
        Some((8, 2, true))
    );
    assert_eq!(
        resolve_rerank_token_usage(None, Some(9)),
        Some((9, 0, true))
    );
    assert_eq!(resolve_rerank_token_usage(None, None), None);
}

#[test]
fn io_reconcile_behaviors() {
    let value = Usage {
        input_tokens: 100,
        output_tokens: 12,
        cached_tokens: Some(25),
        ..Usage::default()
    };
    assert_eq!(
        reconcile_io_usage(&ResponseObject::Usage(value)),
        (75, 12, true)
    );
}

#[test]
fn io_reconcile_boundaries() {
    assert_eq!(
        reconcile_io_usage(&ResponseObject::WithUsage(None)),
        (0, 0, false)
    );
}

#[test]
fn recovered_usage_behaviors() {
    let value = usage();
    assert_eq!(
        recovered_partial_usage_tokens(Some(&value), TokenRateLimitType::Total),
        (90, 70, 20)
    );
    assert_eq!(
        recovered_partial_usage_tokens(None, TokenRateLimitType::Total),
        (0, 0, 0)
    );
}
