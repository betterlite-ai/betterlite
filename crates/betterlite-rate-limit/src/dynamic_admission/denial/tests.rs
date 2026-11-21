#![cfg(test)]

use betterlite_error::proxy_rate_limit_error::map_v3_rate_limit_type;
use betterlite_error::rate_limit::{
    InvalidRateLimitHeader, ProxyRateLimitError, RateLimitDimension,
};
use betterlite_redact::structured::Payload;

use super::{DenialError, OverLimitFacts, build_over_limit_error};
use crate::dynamic_admission::tests::support::{
    MODEL, capture, has_header, header, ok, over_limit, response,
};
use crate::dynamic_priority::descriptors::{PRIORITY_DESCRIPTOR_KEY, SATURATION_DESCRIPTOR_KEY};
use crate::limiter_records::{RateLimitResponse, RateLimitStatus, RateLimitType};

const UNKNOWN_KEY: &str = "some_future_descriptor";

fn over_limit_facts<'facts>(
    model: &'facts str,
    priority: Option<&'facts str>,
) -> OverLimitFacts<'facts> {
    OverLimitFacts {
        model,
        priority,
        saturation: 0.75,
        window_size: 60,
        model_tpm: Some(1000),
        model_rpm: Some(100),
        resolved_model: String::from("gpt-4-0613"),
        llm_provider: String::from("openai"),
    }
}

fn statuses(items: Vec<RateLimitStatus>) -> RateLimitResponse {
    response("OVER_LIMIT", items)
}

fn detail_error(error: &ProxyRateLimitError) -> &Payload {
    let Payload::Object(fields) = &error.detail else {
        panic!("the detail is an object, got {:?}", error.detail);
    };
    fields.get("error").expect("the detail names the error")
}

fn detail_text(error: &ProxyRateLimitError) -> &str {
    match detail_error(error) {
        Payload::String(text) => text,
        other => panic!("the detail error is text, got {other:?}"),
    }
}

#[test]
fn denial_model_capacity_error_carries_the_text_and_headers() {
    let error = build_over_limit_error(
        &statuses(vec![over_limit(
            SATURATION_DESCRIPTOR_KEY,
            RateLimitType::Requests,
        )]),
        &over_limit_facts(MODEL, Some("default")),
    )
    .expect("the known descriptor key names a 429");
    assert_eq!(
        detail_text(&error),
        "Model capacity reached for gpt-4. Priority: default, Rate limit type: requests, Model TPM: 1000, Model RPM: 100, Remaining: 0"
    );
    assert_eq!(error.message, detail_text(&error));
    assert_eq!(header(&error, "retry-after"), "60");
    assert_eq!(header(&error, "rate_limit_type"), "requests");
    assert_eq!(header(&error, "x-litellm-priority"), "default");
    assert!(!has_header(&error, "x-litellm-saturation"));
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Requests));
    assert_eq!(error.model, "gpt-4-0613");
    assert_eq!(error.llm_provider, "openai");
}

#[test]
fn denial_model_capacity_error_prints_not_configured_for_absent_limits() {
    let mut facts = over_limit_facts(MODEL, Some("default"));
    facts.model_tpm = None;
    facts.model_rpm = None;
    let error = build_over_limit_error(
        &statuses(vec![over_limit(
            SATURATION_DESCRIPTOR_KEY,
            RateLimitType::Requests,
        )]),
        &facts,
    )
    .expect("the known descriptor key names a 429");
    assert!(
        detail_text(&error).contains("Model TPM: not configured, Model RPM: not configured"),
        "detail: {}",
        detail_text(&error)
    );
}

#[test]
fn denial_zero_limit_is_a_configured_limit() {
    let mut facts = over_limit_facts(MODEL, Some("default"));
    facts.model_tpm = Some(0);
    let error = build_over_limit_error(
        &statuses(vec![over_limit(
            SATURATION_DESCRIPTOR_KEY,
            RateLimitType::Requests,
        )]),
        &facts,
    )
    .expect("the known descriptor key names a 429");
    assert!(
        detail_text(&error).contains("Model TPM: 0"),
        "detail: {}",
        detail_text(&error)
    );
}

#[test]
fn denial_absent_priority_prints_none_in_the_text_and_default_in_the_header() {
    let error = build_over_limit_error(
        &statuses(vec![over_limit(
            SATURATION_DESCRIPTOR_KEY,
            RateLimitType::Requests,
        )]),
        &over_limit_facts(MODEL, None),
    )
    .expect("the known descriptor key names a 429");
    assert!(
        detail_text(&error).contains("Priority: None, Rate limit type: requests"),
        "detail: {}",
        detail_text(&error)
    );
    assert_eq!(header(&error, "x-litellm-priority"), "default");
}

#[test]
fn denial_empty_priority_prints_nothing_in_the_text_and_default_in_the_header() {
    let error = build_over_limit_error(
        &statuses(vec![over_limit(
            SATURATION_DESCRIPTOR_KEY,
            RateLimitType::Requests,
        )]),
        &over_limit_facts(MODEL, Some("")),
    )
    .expect("the known descriptor key names a 429");
    assert!(
        detail_text(&error).contains("Priority: , Rate limit type: requests"),
        "detail: {}",
        detail_text(&error)
    );
    assert_eq!(header(&error, "x-litellm-priority"), "default");
}

#[test]
fn denial_priority_error_carries_the_saturation_text_and_header() {
    let error = build_over_limit_error(
        &statuses(vec![over_limit(
            PRIORITY_DESCRIPTOR_KEY,
            RateLimitType::Tokens,
        )]),
        &over_limit_facts(MODEL, Some("default")),
    )
    .expect("the known descriptor key names a 429");
    assert_eq!(
        detail_text(&error),
        "Priority-based rate limit exceeded. Model: gpt-4, Priority: default, Rate limit type: tokens, Model TPM: 1000, Model RPM: 100, Remaining: 0, Model saturation: 75.0%"
    );
    assert_eq!(header(&error, "retry-after"), "60");
    assert_eq!(header(&error, "rate_limit_type"), "tokens");
    assert_eq!(header(&error, "x-litellm-priority"), "default");
    assert_eq!(header(&error, "x-litellm-saturation"), "75.00%");
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Tokens));
}

#[test]
fn denial_priority_error_logs_the_enforcement_line() {
    let logs = capture(|| {
        build_over_limit_error(
            &statuses(vec![over_limit(
                PRIORITY_DESCRIPTOR_KEY,
                RateLimitType::Requests,
            )]),
            &over_limit_facts(MODEL, Some("default")),
        )
        .expect("the known descriptor key names a 429");
    });
    assert!(
        logs.contains("Enforcing priority limits for gpt-4, saturation: 75.0%, priority: default"),
        "logs: {logs}"
    );
}

#[test]
fn denial_priority_error_prints_none_for_an_absent_priority() {
    let error = build_over_limit_error(
        &statuses(vec![over_limit(
            PRIORITY_DESCRIPTOR_KEY,
            RateLimitType::Requests,
        )]),
        &over_limit_facts(MODEL, None),
    )
    .expect("the known descriptor key names a 429");
    assert!(
        detail_text(&error).contains("Priority: None, Rate limit type: requests"),
        "detail: {}",
        detail_text(&error)
    );
    assert_eq!(header(&error, "x-litellm-priority"), "default");
}

#[test]
fn denial_max_parallel_type_maps_to_the_concurrent_dimension() {
    let error = build_over_limit_error(
        &statuses(vec![over_limit(
            SATURATION_DESCRIPTOR_KEY,
            RateLimitType::MaxParallelRequests,
        )]),
        &over_limit_facts(MODEL, Some("default")),
    )
    .expect("the known descriptor key names a 429");
    assert_eq!(header(&error, "rate_limit_type"), "max_parallel_requests");
    assert_eq!(
        error.rate_limit_type,
        Some(RateLimitDimension::ConcurrentRequests)
    );
}

#[test]
fn denial_unknown_key_falls_through_to_the_guard() {
    let error = build_over_limit_error(
        &statuses(vec![over_limit(UNKNOWN_KEY, RateLimitType::Tokens)]),
        &over_limit_facts(MODEL, Some("default")),
    )
    .expect("the guard still refuses the request");
    let Payload::Object(fields) = &error.detail else {
        panic!("the guard detail is an object, got {:?}", error.detail);
    };
    assert_eq!(
        fields.get("error"),
        Some(&Payload::String(String::from("Rate limit exceeded")))
    );
    assert_eq!(
        fields.get("descriptor_key"),
        Some(&Payload::String(String::from(UNKNOWN_KEY)))
    );
    assert_eq!(
        fields.get("rate_limit_type"),
        Some(&Payload::String(String::from("tokens")))
    );
    assert_eq!(header(&error, "retry-after"), "60");
    assert_eq!(header(&error, "x-litellm-priority"), "default");
    assert!(!has_header(&error, "rate_limit_type"));
    assert!(!has_header(&error, "x-litellm-saturation"));
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Tokens));
    assert_eq!(error.model, "gpt-4-0613");
    assert_eq!(error.llm_provider, "openai");
}

#[test]
fn denial_unknown_key_then_known_key_raises_the_known_error() {
    let error = build_over_limit_error(
        &statuses(vec![
            over_limit(UNKNOWN_KEY, RateLimitType::Tokens),
            over_limit(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests),
        ]),
        &over_limit_facts(MODEL, Some("default")),
    )
    .expect("the loop goes on to the known key");
    assert!(
        detail_text(&error).starts_with("Model capacity reached for gpt-4."),
        "detail: {}",
        detail_text(&error)
    );
}

#[test]
fn denial_known_key_then_unknown_key_raises_the_known_error() {
    let error = build_over_limit_error(
        &statuses(vec![
            over_limit(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests),
            over_limit(UNKNOWN_KEY, RateLimitType::Tokens),
        ]),
        &over_limit_facts(MODEL, Some("default")),
    )
    .expect("the first known key names the 429");
    assert!(
        detail_text(&error).starts_with("Model capacity reached for gpt-4."),
        "detail: {}",
        detail_text(&error)
    );
}

#[test]
fn denial_non_over_limit_statuses_are_skipped() {
    let error = build_over_limit_error(
        &response(
            "OVER_LIMIT",
            vec![
                ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests),
                ok(PRIORITY_DESCRIPTOR_KEY, RateLimitType::Requests),
            ],
        ),
        &over_limit_facts(MODEL, Some("default")),
    )
    .expect("the guard refuses an unexplained OVER_LIMIT");
    let Payload::Object(fields) = &error.detail else {
        panic!("the guard detail is an object, got {:?}", error.detail);
    };
    assert_eq!(
        fields.get("descriptor_key"),
        Some(&Payload::String(String::from("unknown")))
    );
    assert_eq!(
        fields.get("rate_limit_type"),
        Some(&Payload::String(String::from("unknown")))
    );
    assert_eq!(error.rate_limit_type, None);
}

#[test]
fn denial_guard_prefers_the_first_over_limit_status_for_the_detail() {
    let error = build_over_limit_error(
        &statuses(vec![
            over_limit("first_unknown", RateLimitType::Requests),
            over_limit("second_unknown", RateLimitType::Tokens),
        ]),
        &over_limit_facts(MODEL, Some("default")),
    )
    .expect("the guard refuses the request");
    let Payload::Object(fields) = &error.detail else {
        panic!("the guard detail is an object, got {:?}", error.detail);
    };
    assert_eq!(
        fields.get("descriptor_key"),
        Some(&Payload::String(String::from("first_unknown")))
    );
    assert_eq!(
        fields.get("rate_limit_type"),
        Some(&Payload::String(String::from("requests")))
    );
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Requests));
}

#[test]
fn denial_guard_without_statuses_reports_unknown() {
    let error = build_over_limit_error(
        &response("OVER_LIMIT", Vec::new()),
        &over_limit_facts(MODEL, None),
    )
    .expect("the guard refuses the request");
    let Payload::Object(fields) = &error.detail else {
        panic!("the guard detail is an object, got {:?}", error.detail);
    };
    assert_eq!(
        fields.get("descriptor_key"),
        Some(&Payload::String(String::from("unknown")))
    );
    assert_eq!(
        fields.get("rate_limit_type"),
        Some(&Payload::String(String::from("unknown")))
    );
    assert_eq!(header(&error, "retry-after"), "60");
    assert_eq!(header(&error, "x-litellm-priority"), "default");
    assert_eq!(error.rate_limit_type, map_v3_rate_limit_type(None));
}

#[test]
fn denial_guard_logs_the_refusal_with_the_response() {
    let logs = capture(|| {
        build_over_limit_error(
            &statuses(vec![over_limit(UNKNOWN_KEY, RateLimitType::Requests)]),
            &over_limit_facts(MODEL, Some("default")),
        )
        .expect("the guard refuses the request");
    });
    assert!(
        logs.contains("OVER_LIMIT response with unknown descriptor_key(s) - refusing request"),
        "logs: {logs}"
    );
    assert!(
        logs.contains("response={\"overall_code\":\"OVER_LIMIT\""),
        "logs: {logs}"
    );
}

#[test]
fn denial_unencodable_priority_drops_only_its_header() {
    let logs = capture(|| {
        let error = build_over_limit_error(
            &statuses(vec![over_limit(
                SATURATION_DESCRIPTOR_KEY,
                RateLimitType::Requests,
            )]),
            &over_limit_facts(MODEL, Some("bad\npriority")),
        )
        .expect("the 429 survives a priority no header can carry");
        assert!(!has_header(&error, "x-litellm-priority"));
        assert_eq!(header(&error, "retry-after"), "60");
        assert_eq!(header(&error, "rate_limit_type"), "requests");
        assert!(
            detail_text(&error).contains("Priority: bad\npriority,"),
            "detail: {}",
            detail_text(&error)
        );
    });
    assert!(
        logs.contains("Dropping the x-litellm-priority header"),
        "logs: {logs}"
    );
}

#[test]
fn denial_denial_error_converts_the_header_failure() {
    let failure = DenialError::from(InvalidRateLimitHeader::Value {
        name: String::from("x-litellm-priority"),
    });
    assert!(matches!(failure, DenialError::Header(_)));
}
