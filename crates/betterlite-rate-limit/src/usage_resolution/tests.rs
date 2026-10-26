#![cfg(test)]

use std::collections::BTreeMap;

use super::{
    ResponseObject, Usage, aggregate_only_total_tokens, get_rate_limit_type,
    get_total_tokens_from_usage, recovered_partial_usage_tokens, resolve_io_token_reconcile_usage,
    resolve_rerank_token_usage, response_usage,
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
    let mut settings = BTreeMap::new();
    assert_eq!(get_rate_limit_type(&settings), "total");
    settings.insert("token_rate_limit_type".to_owned(), "input".to_owned());
    assert_eq!(get_rate_limit_type(&settings), "input");
    settings.insert("token_rate_limit_type".to_owned(), "invalid".to_owned());
    assert_eq!(get_rate_limit_type(&settings), "total");
}

#[test]
fn usage_filtering_behaviors() {
    let value = usage();
    assert_eq!(get_total_tokens_from_usage(Some(&value), "output"), 20);
    assert_eq!(get_total_tokens_from_usage(Some(&value), "input"), 70);
    assert_eq!(get_total_tokens_from_usage(Some(&value), "total"), 90);
    assert_eq!(
        aggregate_only_total_tokens(Some(&Usage {
            total_tokens: 120,
            ..Usage::default()
        })),
        120
    );
    assert_eq!(aggregate_only_total_tokens(Some(&value)), 0);
    assert_eq!(
        response_usage(&ResponseObject::Mapping(value.clone())),
        Some(&value)
    );
}

#[test]
fn usage_filtering_boundaries() {
    assert_eq!(get_total_tokens_from_usage(None, "total"), 0);
    assert_eq!(
        get_total_tokens_from_usage(
            Some(&Usage {
                total_tokens: 4,
                ..Usage::default()
            }),
            "total"
        ),
        4
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
        resolve_io_token_reconcile_usage(&ResponseObject::Usage(value)),
        (75, 12, true)
    );
}

#[test]
fn io_reconcile_boundaries() {
    assert_eq!(
        resolve_io_token_reconcile_usage(&ResponseObject::WithUsage(None)),
        (0, 0, false)
    );
}

#[test]
fn recovered_usage_behaviors() {
    let value = usage();
    assert_eq!(
        recovered_partial_usage_tokens(Some(&value), "total"),
        (90, 70, 20)
    );
    assert_eq!(recovered_partial_usage_tokens(None, "total"), (0, 0, 0));
}
