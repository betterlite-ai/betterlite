#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use tokio::runtime::Builder;

use super::{AdmissionKey, CheckRateLimitError, RateLimitCheck};
use crate::atomic_counter::payload::IncrementAmounts;
use crate::dynamic_admission::limiter_port::AdmissionLimiterError;
use crate::dynamic_admission::tests::support::{
    AdmissionSetup, Canned, Harness, MODEL, WINDOW, capture, header, key, key_with_span, limits,
    ok, over_limit, percent, policy, response, settlement_failure, stored_response,
    unverifiable_failure,
};
use crate::dynamic_priority::descriptors::{PRIORITY_DESCRIPTOR_KEY, SATURATION_DESCRIPTOR_KEY};
use crate::dynamic_priority::settings::PriorityPolicy;
use crate::limiter_records::{
    RateLimitDescriptor, RateLimitDescriptorRateLimit, RateLimitResponse, RateLimitType,
    ReservationBackend, ReservationWindowIdentity,
};
use crate::request_stash::RequestStashSlot;

const GROUP_RPM: i64 = 100;
const GROUP_TPM: i64 = 1000;

fn setup(policy: PriorityPolicy, atomic: Canned, tracking: Canned) -> AdmissionSetup {
    AdmissionSetup {
        policy,
        router: None,
        seeded: Vec::new(),
        atomic,
        tracking,
    }
}

fn reserved_policy() -> PriorityPolicy {
    policy(0.5, Some(&[("default", percent(0.5))]))
}

fn model_wide_descriptor() -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: String::from(SATURATION_DESCRIPTOR_KEY),
        value: String::from(MODEL),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: Some(GROUP_RPM),
            tokens_per_unit: Some(GROUP_TPM),
            max_parallel_requests: None,
            window_size: Some(WINDOW),
        }),
    }
}

fn priority_descriptor(value: &str, requests: i64, tokens: i64) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: String::from(PRIORITY_DESCRIPTOR_KEY),
        value: value.to_owned(),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: Some(requests),
            tokens_per_unit: Some(tokens),
            max_parallel_requests: None,
            window_size: Some(WINDOW),
        }),
    }
}

fn check<'check>(
    admission_key: &'check AdmissionKey<'check>,
    stash: &'check RequestStashSlot,
    priority: Option<&'check str>,
    saturation: f64,
) -> RateLimitCheck<'check> {
    RateLimitCheck {
        model: MODEL,
        model_group_info: limits(Some(GROUP_RPM), Some(GROUP_TPM)),
        key: admission_key,
        priority,
        saturation,
        stash,
    }
}

fn empty_tracking() -> Canned {
    Ok(response("OK", Vec::new()))
}

fn reservation_windows() -> BTreeSet<ReservationWindowIdentity> {
    BTreeSet::from([ReservationWindowIdentity {
        counter_key: String::from("{model_saturation_check:gpt-4}:requests"),
        window_start: String::from("1700000000"),
        backend: ReservationBackend::Redis,
    }])
}

#[tokio::test]
async fn admission_model_wide_descriptor_uses_multiplier_one() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(response(
            "OK",
            vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
        )),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.3))
        .await
        .expect("the atomic answer allows the request");
    let calls = harness.limiter.atomic_calls();
    assert_eq!(calls[0].descriptors[0], model_wide_descriptor());
    assert_eq!(
        calls[0].descriptors[0]
            .rate_limit
            .as_ref()
            .expect("the model-wide descriptor carries limits"),
        &RateLimitDescriptorRateLimit {
            requests_per_unit: Some(GROUP_RPM),
            tokens_per_unit: Some(GROUP_TPM),
            max_parallel_requests: None,
            window_size: Some(WINDOW),
        }
    );
}

#[test]
fn admission_logs_the_atomic_response() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(response(
            "OK",
            vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
        )),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    let request = check(&admission_key, &stash, Some("default"), 0.3);
    let logs = capture(|| {
        let runtime = Builder::new_current_thread()
            .build()
            .expect("the test runtime starts");
        runtime
            .block_on(harness.admission.check_rate_limits(&request))
            .expect("the atomic answer allows the request");
    });
    assert!(
        logs.contains("Atomic check+increment response:"),
        "logs: {logs}"
    );
}

#[tokio::test]
async fn admission_below_threshold_sends_only_the_model_wide_descriptor() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(response(
            "OK",
            vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
        )),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.3))
        .await
        .expect("the atomic answer allows the request");
    let calls = harness.limiter.atomic_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].descriptors, vec![model_wide_descriptor()]);
    assert_eq!(
        calls[0].increments,
        vec![IncrementAmounts {
            requests: 1,
            tokens: 0
        }]
    );
}

#[tokio::test]
async fn admission_below_threshold_tracks_the_priority_descriptor() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(response(
            "OK",
            vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
        )),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.3))
        .await
        .expect("the atomic answer allows the request");
    let calls = harness.limiter.tracking_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].descriptors,
        vec![priority_descriptor("gpt-4:default", 50, 500)]
    );
    assert!(!calls[0].read_only);
}

#[tokio::test]
async fn admission_at_threshold_enforces_the_priority_descriptor() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(response(
            "OK",
            vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
        )),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.5))
        .await
        .expect("the atomic answer allows the request");
    let calls = harness.limiter.atomic_calls();
    assert_eq!(
        calls[0].descriptors,
        vec![
            model_wide_descriptor(),
            priority_descriptor("gpt-4:default", 50, 500)
        ]
    );
    assert_eq!(calls[0].increments.len(), 2);
    assert!(harness.limiter.tracking_calls().is_empty());
}

#[tokio::test]
async fn admission_above_threshold_enforces_the_priority_descriptor() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(response(
            "OK",
            vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
        )),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.9))
        .await
        .expect("the atomic answer allows the request");
    let calls = harness.limiter.atomic_calls();
    assert_eq!(calls[0].descriptors.len(), 2);
    assert_eq!(calls[0].increments.len(), 2);
    assert!(harness.limiter.tracking_calls().is_empty());
}

#[tokio::test]
async fn admission_tracking_statuses_follow_the_atomic_statuses() {
    let atomic_status = ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests);
    let tracking_status = over_limit(PRIORITY_DESCRIPTOR_KEY, RateLimitType::Requests);
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(response("OK", vec![atomic_status.clone()])),
        Ok(response("OVER_LIMIT", vec![tracking_status.clone()])),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.3))
        .await
        .expect("the tracking OVER_LIMIT is dropped");
    let stored = stored_response(&stash).expect("the answer is stored for the post-call side");
    assert_eq!(stored.overall_code, "OK");
    assert_eq!(stored.statuses, vec![atomic_status, tracking_status]);
}

#[tokio::test]
async fn admission_stores_the_atomic_answer_unchanged_when_enforced() {
    let atomic = response(
        "OK",
        vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
    );
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(atomic.clone()),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.9))
        .await
        .expect("the atomic answer allows the request");
    assert_eq!(stored_response(&stash), Some(atomic));
}

#[tokio::test]
async fn admission_keeps_the_atomic_reservation_windows_when_enforced() {
    let atomic = RateLimitResponse {
        overall_code: String::from("OK"),
        statuses: vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
        reservation_windows: Some(reservation_windows()),
    };
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(atomic.clone()),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.9))
        .await
        .expect("the atomic answer allows the request");
    assert_eq!(stored_response(&stash), Some(atomic));
}

#[tokio::test]
async fn admission_drops_the_stored_reservation_windows_when_tracking() {
    let atomic = RateLimitResponse {
        overall_code: String::from("OK"),
        statuses: vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
        reservation_windows: Some(reservation_windows()),
    };
    let tracking = RateLimitResponse {
        overall_code: String::from("OK"),
        statuses: Vec::new(),
        reservation_windows: Some(reservation_windows()),
    };
    let harness = Harness::build(setup(reserved_policy(), Ok(atomic), Ok(tracking)));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.3))
        .await
        .expect("the atomic answer allows the request");
    let stored = stored_response(&stash).expect("the answer is stored for the post-call side");
    assert_eq!(stored.overall_code, "OK");
    assert!(stored.reservation_windows.is_none());
}

#[tokio::test]
async fn admission_no_priority_descriptors_stores_the_atomic_answer() {
    let atomic = response(
        "OK",
        vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
    );
    let harness = Harness::build(setup(
        policy(0.5, None),
        Ok(atomic.clone()),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, None, 0.1))
        .await
        .expect("the atomic answer allows the request");
    assert!(harness.limiter.tracking_calls().is_empty());
    assert_eq!(harness.limiter.atomic_calls()[0].descriptors.len(), 1);
    assert_eq!(stored_response(&stash), Some(atomic));
}

#[tokio::test]
async fn admission_over_limit_returns_the_429() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(response(
            "OVER_LIMIT",
            vec![over_limit(
                SATURATION_DESCRIPTOR_KEY,
                RateLimitType::Requests,
            )],
        )),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    let failure = harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.9))
        .await
        .expect_err("the model-wide counter overflowed");
    let rejection = match failure {
        CheckRateLimitError::Rejection(error) => error,
        other => panic!("the known status names a 429, got {other:?}"),
    };
    assert_eq!(
        rejection.message,
        "Model capacity reached for gpt-4. Priority: default, Rate limit type: requests, Model TPM: 1000, Model RPM: 100, Remaining: 0"
    );
    assert_eq!(header(&rejection, "retry-after"), "60");
    assert!(harness.limiter.tracking_calls().is_empty());
    assert!(stash.get().is_none());
}

#[tokio::test]
async fn admission_priority_over_limit_names_the_priority_error() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(response(
            "OVER_LIMIT",
            vec![over_limit(PRIORITY_DESCRIPTOR_KEY, RateLimitType::Tokens)],
        )),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    let failure = harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.5))
        .await
        .expect_err("the priority pool overflowed");
    let rejection = match failure {
        CheckRateLimitError::Rejection(error) => error,
        other => panic!("the known status names a 429, got {other:?}"),
    };
    assert_eq!(header(&rejection, "rate_limit_type"), "tokens");
    assert_eq!(header(&rejection, "x-litellm-saturation"), "50.00%");
}

#[tokio::test]
async fn admission_over_limit_resolves_the_provider_once() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(response(
            "OVER_LIMIT",
            vec![over_limit(
                SATURATION_DESCRIPTOR_KEY,
                RateLimitType::Requests,
            )],
        )),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    let failure = harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.9))
        .await
        .expect_err("the model-wide counter overflowed");
    let rejection = match failure {
        CheckRateLimitError::Rejection(error) => error,
        other => panic!("the known status names a 429, got {other:?}"),
    };
    assert_eq!(rejection.model, "gpt-4-0613");
    assert_eq!(rejection.llm_provider, "openai");
    assert_eq!(harness.providers.calls(), vec![String::from(MODEL)]);
}

#[tokio::test]
async fn admission_allowing_path_resolves_no_provider() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(response(
            "OK",
            vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
        )),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.3))
        .await
        .expect("the atomic answer allows the request");
    assert!(harness.providers.calls().is_empty());
}

#[tokio::test]
async fn admission_retry_after_comes_from_the_limiter_window() {
    let harness = Harness::build_at(
        setup(
            reserved_policy(),
            Ok(response(
                "OVER_LIMIT",
                vec![over_limit(
                    SATURATION_DESCRIPTOR_KEY,
                    RateLimitType::Requests,
                )],
            )),
            empty_tracking(),
        ),
        120,
    );
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    let failure = harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.9))
        .await
        .expect_err("the model-wide counter overflowed");
    let rejection = match failure {
        CheckRateLimitError::Rejection(error) => error,
        other => panic!("the known status names a 429, got {other:?}"),
    };
    assert_eq!(header(&rejection, "retry-after"), "120");
    assert_eq!(
        harness.limiter.atomic_calls()[0].descriptors[0]
            .rate_limit
            .as_ref()
            .expect("the descriptor carries limits")
            .window_size,
        Some(WINDOW)
    );
}

#[tokio::test]
async fn admission_limiter_failure_returns_the_settlement_error() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Err(settlement_failure()),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    let failure = harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.3))
        .await
        .expect_err("the atomic call failed");
    assert!(matches!(
        failure,
        CheckRateLimitError::Limiter(AdmissionLimiterError::Settlement(_))
    ));
    assert!(stash.get().is_none());
}

#[tokio::test]
async fn admission_fail_closed_error_keeps_its_variant() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Err(unverifiable_failure()),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    let failure = harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.3))
        .await
        .expect_err("the atomic call could not be verified");
    assert!(matches!(
        failure,
        CheckRateLimitError::Limiter(AdmissionLimiterError::Unverifiable(_))
    ));
    assert!(stash.get().is_none());
}

#[tokio::test]
async fn admission_tracking_failure_returns_the_error_and_stores_nothing() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(response(
            "OK",
            vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
        )),
        Err(settlement_failure()),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key(&metadata);
    let stash = RequestStashSlot::default();
    let failure = harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.3))
        .await
        .expect_err("the tracking call failed");
    assert!(matches!(
        failure,
        CheckRateLimitError::Limiter(AdmissionLimiterError::Settlement(_))
    ));
    assert!(stash.get().is_none());
}

#[tokio::test]
async fn admission_parent_span_reaches_both_limiter_calls() {
    let harness = Harness::build(setup(
        reserved_policy(),
        Ok(response(
            "OK",
            vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
        )),
        empty_tracking(),
    ));
    let metadata = BTreeMap::new();
    let admission_key = key_with_span(&metadata, ParentOtelSpan::Opaque);
    let stash = RequestStashSlot::default();
    harness
        .admission
        .check_rate_limits(&check(&admission_key, &stash, Some("default"), 0.3))
        .await
        .expect("the atomic answer allows the request");
    assert!(harness.limiter.atomic_calls()[0].parent_span);
    assert!(harness.limiter.tracking_calls()[0].parent_span);
}
