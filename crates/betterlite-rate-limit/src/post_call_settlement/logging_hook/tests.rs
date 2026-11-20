#![cfg(test)]

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;

use crate::limiter_records::RateLimitType;
use crate::post_call_settlement::logging_hook::{LoggingTargets, logging_hook};
use crate::post_call_settlement::tests::support::{
    headers, install, number, rate_limit_response, status,
};
use crate::request_stash::RequestStashSlot;

#[test]
fn mirrors_the_statuses_into_both_targets() {
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.rate_limit_response = Some(rate_limit_response(vec![status(
            "user",
            RateLimitType::Tokens,
            1000,
            750,
        )]));
    });
    let mut standard_logging = headers(&[("x-request-id", Json::Text("abc".to_owned()))]);
    let mut response_hidden = AdditionalHeaders::new();
    logging_hook(
        &slot,
        None,
        LoggingTargets {
            standard_logging_object: Some(&mut standard_logging),
            response_hidden_params: Some(&mut response_hidden),
        },
    );
    assert_eq!(
        standard_logging.get("x-ratelimit-user-remaining-tokens"),
        Some(&number(750))
    );
    assert_eq!(
        standard_logging.get("x-ratelimit-user-limit-tokens"),
        Some(&number(1000))
    );
    assert_eq!(
        standard_logging.get("x-request-id"),
        Some(&Json::Text("abc".to_owned()))
    );
    assert_eq!(
        response_hidden.get("x-ratelimit-user-remaining-tokens"),
        Some(&number(750))
    );
    assert_eq!(
        response_hidden.get("x-ratelimit-user-limit-tokens"),
        Some(&number(1000))
    );
}

#[test]
fn mirrors_into_a_lone_response_target() {
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.rate_limit_response = Some(rate_limit_response(vec![status(
            "user",
            RateLimitType::Tokens,
            1000,
            750,
        )]));
    });
    let mut response_hidden = AdditionalHeaders::new();
    logging_hook(
        &slot,
        None,
        LoggingTargets {
            standard_logging_object: None,
            response_hidden_params: Some(&mut response_hidden),
        },
    );
    assert_eq!(response_hidden.len(), 2);
    assert_eq!(
        response_hidden.get("x-ratelimit-user-remaining-tokens"),
        Some(&number(750))
    );
}

#[test]
fn leaves_both_targets_alone_without_a_stash() {
    let slot = RequestStashSlot::default();
    let mut standard_logging = headers(&[("x-request-id", Json::Text("abc".to_owned()))]);
    let mut response_hidden = AdditionalHeaders::new();
    logging_hook(
        &slot,
        None,
        LoggingTargets {
            standard_logging_object: Some(&mut standard_logging),
            response_hidden_params: Some(&mut response_hidden),
        },
    );
    assert_eq!(standard_logging.len(), 1);
    assert!(response_hidden.is_empty());
}

#[test]
fn leaves_both_targets_alone_without_statuses() {
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.rate_limit_response = Some(rate_limit_response(Vec::new()));
    });
    let mut standard_logging = headers(&[("x-request-id", Json::Text("abc".to_owned()))]);
    let mut response_hidden = AdditionalHeaders::new();
    logging_hook(
        &slot,
        None,
        LoggingTargets {
            standard_logging_object: Some(&mut standard_logging),
            response_hidden_params: Some(&mut response_hidden),
        },
    );
    assert_eq!(standard_logging.len(), 1);
    assert!(response_hidden.is_empty());
}

#[test]
fn mirrors_for_the_call_that_owns_the_stash() {
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.owner_litellm_call_id = Some("call-a".to_owned());
        state.rate_limit_response = Some(rate_limit_response(vec![status(
            "user",
            RateLimitType::Tokens,
            1000,
            750,
        )]));
    });
    let mut standard_logging = AdditionalHeaders::new();
    logging_hook(
        &slot,
        Some("call-a"),
        LoggingTargets {
            standard_logging_object: Some(&mut standard_logging),
            response_hidden_params: None,
        },
    );
    assert_eq!(standard_logging.len(), 2);
}

#[test]
fn skips_a_stash_owned_by_another_call() {
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.owner_litellm_call_id = Some("call-a".to_owned());
        state.rate_limit_response = Some(rate_limit_response(vec![status(
            "user",
            RateLimitType::Tokens,
            1000,
            750,
        )]));
    });
    let mut standard_logging = AdditionalHeaders::new();
    logging_hook(
        &slot,
        Some("call-b"),
        LoggingTargets {
            standard_logging_object: Some(&mut standard_logging),
            response_hidden_params: None,
        },
    );
    assert!(standard_logging.is_empty());
}

#[test]
fn mirrors_when_the_callback_carries_no_call_id() {
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.owner_litellm_call_id = Some("call-a".to_owned());
        state.rate_limit_response = Some(rate_limit_response(vec![status(
            "user",
            RateLimitType::Tokens,
            1000,
            750,
        )]));
    });
    let mut standard_logging = AdditionalHeaders::new();
    logging_hook(
        &slot,
        None,
        LoggingTargets {
            standard_logging_object: Some(&mut standard_logging),
            response_hidden_params: None,
        },
    );
    assert_eq!(standard_logging.len(), 2);
}

#[test]
fn writes_the_same_headers_when_the_hook_runs_twice() {
    let slot = RequestStashSlot::default();
    install(&slot, |state| {
        state.rate_limit_response = Some(rate_limit_response(vec![status(
            "user",
            RateLimitType::Tokens,
            1000,
            750,
        )]));
    });
    let mut standard_logging = headers(&[("x-request-id", Json::Text("abc".to_owned()))]);
    logging_hook(
        &slot,
        None,
        LoggingTargets {
            standard_logging_object: Some(&mut standard_logging),
            response_hidden_params: None,
        },
    );
    logging_hook(
        &slot,
        None,
        LoggingTargets {
            standard_logging_object: Some(&mut standard_logging),
            response_hidden_params: None,
        },
    );
    assert_eq!(standard_logging.len(), 3);
    assert_eq!(
        standard_logging.get("x-ratelimit-user-remaining-tokens"),
        Some(&number(750))
    );
}
