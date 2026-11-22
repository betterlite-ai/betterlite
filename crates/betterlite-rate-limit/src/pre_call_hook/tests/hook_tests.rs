#![cfg(test)]

use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_core::routes::CallType;
use betterlite_error::rate_limit::RateLimitDimension;

use super::support::{
    self, BatchLimiter, Fixture, Parts, StashSnapshot, count, entries, parallel_key,
    proxy_rate_limit, requests_key, scopes, snapshot, token_window,
};
use crate::limiter_records::{RateLimitResponse, RateLimitType, RateLimitedModel};
use crate::pre_call_hook::{PreCallHookError, PreCallHookRejection};
use crate::project_admission::ProjectAdmissionError;
use crate::request_stash::RequestStashSlot;
use crate::token_reservations::TokenReservationError;

#[tokio::test]
async fn hook_hands_a_batch_call_to_the_batch_limiter() {
    let batch = BatchLimiter::succeeding();
    let mut parts = Parts::new(Vec::new(), Vec::new(), None);
    parts.batch_limiter = Some(Arc::clone(&batch));
    let fixture = Fixture::new(&parts);
    let auth = support::auth(r#"{"api_key": "key-1", "rpm_limit": 10}"#);
    let mut data = support::data(r#"{"model": "gpt4"}"#);

    fixture
        .run(None, &auth, &mut data, CallType::AcreateBatch)
        .await
        .expect("the batch call passes");

    assert_eq!(batch.seen(), vec![CallType::AcreateBatch]);
}

#[tokio::test]
async fn hook_returns_the_batch_limiter_error_unchanged() {
    let batch = BatchLimiter::failing(proxy_rate_limit("batch is full"));
    let mut parts = Parts::new(Vec::new(), Vec::new(), None);
    parts.batch_limiter = Some(batch);
    let fixture = Fixture::new(&parts);
    let auth = support::auth(r#"{"api_key": "key-1", "rpm_limit": 10}"#);
    let mut data = support::data(r#"{"model": "gpt4"}"#);

    let Err(error) = fixture
        .run(None, &auth, &mut data, CallType::AcreateBatch)
        .await
    else {
        panic!("the batch limiter rejection surfaces");
    };

    assert!(matches!(&error, PreCallHookError::RateLimited(_)));
    let Some(PreCallHookRejection::RateLimited(rate_limited)) = error.rejection() else {
        panic!("the batch rejection is a 429");
    };
    assert_eq!(rate_limited.message, "batch is full");
}

#[tokio::test]
async fn hook_refuses_a_batch_call_without_a_batch_limiter() {
    let fixture = Fixture::plain();
    let auth = support::auth(r#"{"api_key": "key-1", "rpm_limit": 10}"#);
    let mut data = support::data(r#"{"model": "gpt4"}"#);

    let Err(error) = fixture
        .run(None, &auth, &mut data, CallType::AcreateBatch)
        .await
    else {
        panic!("the missing batch limiter is refused");
    };

    assert!(matches!(
        &error,
        PreCallHookError::MissingBatchLimiter {
            call_type: CallType::AcreateBatch
        }
    ));
    assert!(error.rejection().is_none());
}

#[tokio::test]
async fn hook_with_no_descriptors_leaves_the_stash_empty() {
    let fixture = Fixture::plain();
    let auth = support::auth(r#"{"api_key": "key-1"}"#);
    let mut data = support::data(r#"{"model": "gpt4"}"#);
    let slot = RequestStashSlot::default();

    fixture
        .run(Some(&slot), &auth, &mut data, CallType::Acompletion)
        .await
        .expect("a request without limits passes");

    let state = snapshot(&slot);
    assert!(state.response.is_none());
    assert!(state.slot.is_none());
    assert_eq!(state.reserved_tokens, 0);
    assert!(state.tpm_limited_tags.is_empty());
    assert!(fixture.recorder.effects().is_empty());
}

#[tokio::test]
async fn hook_first_pass_over_limit_raises_and_saves_nothing() {
    let fixture = Fixture::plain();
    let auth = support::auth(r#"{"api_key": "key-1", "rpm_limit": 0}"#);
    let mut data = support::data(r#"{"model": "gpt4"}"#);
    let slot = RequestStashSlot::default();

    let Err(error) = fixture
        .run(Some(&slot), &auth, &mut data, CallType::Acompletion)
        .await
    else {
        panic!("the first pass is over the request limit");
    };

    assert!(matches!(&error, PreCallHookError::RateLimited(_)));
    let Some(PreCallHookRejection::RateLimited(rate_limited)) = error.rejection() else {
        panic!("the rejection is a 429");
    };
    assert!(
        rate_limited
            .message
            .starts_with("Rate limit exceeded for api_key: key-1")
    );
    assert_eq!(
        rate_limited.rate_limit_type,
        Some(RateLimitDimension::Requests)
    );
    let state = snapshot(&slot);
    assert!(state.response.is_none());
    assert!(state.slot.is_none());
}

#[tokio::test]
async fn hook_saves_the_response_and_slot_after_a_passing_first_pass() {
    let fixture = Fixture::plain();
    let auth = support::auth(r#"{"api_key": "key-1", "rpm_limit": 5, "max_parallel_requests": 2}"#);
    let mut data = support::data(r#"{"model": "gpt4"}"#);
    let slot = RequestStashSlot::default();
    let other = RequestStashSlot::default();

    fixture
        .run(Some(&slot), &auth, &mut data, CallType::Acompletion)
        .await
        .expect("the request passes");

    let state = snapshot(&slot);
    let response = state.response.expect("the first pass stored its response");
    assert_eq!(response.overall_code, "OK");
    let acquisition = state.slot.expect("the first pass stored its slot");
    assert_eq!(acquisition.slot_id, "slot-1");
    assert_eq!(
        acquisition.counter_keys,
        vec![parallel_key("api_key", "key-1")]
    );
    assert_eq!(
        fixture.recorder.counter(&requests_key("api_key", "key-1")),
        Some(count(1))
    );
    let registry = fixture
        .recorder
        .registry(&parallel_key("api_key", "key-1"))
        .expect("the gauge holds a slot registry");
    assert!(registry.contains_key("slot-1"));
    assert!(
        other.get().is_none(),
        "the hook only wrote the slot it was given"
    );
}

#[tokio::test]
async fn hook_reuses_the_stash_across_calls_of_one_request() {
    let fixture = Fixture::plain();
    let auth = support::auth(r#"{"api_key": "key-1", "rpm_limit": 5}"#);
    let mut data = support::data(r#"{"model": "gpt4"}"#);
    let slot = RequestStashSlot::default();

    fixture
        .run(Some(&slot), &auth, &mut data, CallType::Acompletion)
        .await
        .expect("the first call passes");
    let first = slot.get().expect("the first call claimed the stash");
    fixture
        .run(Some(&slot), &auth, &mut data, CallType::Acompletion)
        .await
        .expect("the second call passes");
    let second = slot.get().expect("the second call keeps the stash");

    assert!(Arc::ptr_eq(&first, &second));
}

#[tokio::test]
async fn hook_skips_the_project_token_limit_in_the_first_pass_with_reservation_on() {
    let mut parts = Parts::new(
        entries([token_window(
            "model_per_project_itpm",
            "project-1:gpt-4",
            11,
        )]),
        Vec::new(),
        None,
    );
    parts.groups = vec![(String::from("gpt4"), String::from("gpt-4"))];
    let fixture = Fixture::new(&parts);
    let auth = support::auth(
        r#"{"api_key": "key-1", "project_id": "project-1",
            "project_metadata": {"model_itpm_limit": {"gpt-4": 11},
                                 "model_otpm_limit": {"gpt-4": 11}}}"#,
    );
    let mut data = support::data(
        r#"{"model": "gpt4", "messages": [{"role": "user", "content": "hello there"}]}"#,
    );
    let slot = RequestStashSlot::default();

    let Err(error) = fixture
        .run(Some(&slot), &auth, &mut data, CallType::Acompletion)
        .await
    else {
        panic!("the project input reservation rejects the request");
    };

    assert!(matches!(
        &error,
        PreCallHookError::Project(ProjectAdmissionError::RateLimited(_))
    ));
    assert!(
        snapshot(&slot).response.is_some(),
        "the first pass stored its response instead of rejecting"
    );
}

#[tokio::test]
async fn hook_leaves_the_project_limits_out_of_the_first_pass_with_reservation_off() {
    let mut parts = Parts::new(
        entries([token_window(
            "model_per_project_itpm",
            "project-1:gpt-4",
            11,
        )]),
        Vec::new(),
        None,
    );
    parts.reservation_off = true;
    parts.groups = vec![(String::from("gpt4"), String::from("gpt-4"))];
    let fixture = Fixture::new(&parts);
    let auth = support::auth(
        r#"{"api_key": "key-1", "project_id": "project-1",
            "project_metadata": {"model_itpm_limit": {"gpt-4": 11},
                                 "model_otpm_limit": {"gpt-4": 11}}}"#,
    );
    let mut data = support::data(
        r#"{"model": "gpt4", "messages": [{"role": "user", "content": "hello there"}]}"#,
    );

    let Err(error) = fixture
        .run(None, &auth, &mut data, CallType::Acompletion)
        .await
    else {
        panic!("the project input reservation rejects the request");
    };

    assert!(matches!(
        &error,
        PreCallHookError::Project(ProjectAdmissionError::RateLimited(_))
    ));
    let Some(PreCallHookRejection::RateLimited(rate_limited)) = error.rejection() else {
        panic!("the rejection is a 429");
    };
    assert!(rate_limited.message.contains("model_per_project_itpm"));
}

#[tokio::test]
async fn hook_releases_the_stashed_slot_when_the_token_reservation_is_over_the_limit() {
    let fixture = Fixture::new(&Parts::new(
        entries([token_window("api_key", "key-1", 5)]),
        Vec::new(),
        None,
    ));
    let auth = support::auth(r#"{"api_key": "key-1", "tpm_limit": 5, "max_parallel_requests": 2}"#);
    let mut data = support::data(
        r#"{"model": "gpt4", "max_tokens": 10, "messages": [{"role": "user", "content": "hi"}]}"#,
    );
    let slot = RequestStashSlot::default();

    let Err(error) = fixture
        .run(Some(&slot), &auth, &mut data, CallType::Acompletion)
        .await
    else {
        panic!("the token reservation is over its limit");
    };

    assert_token_reservation_rejection(&error);
    let state = snapshot(&slot);
    assert!(
        state.slot.is_none(),
        "the rejected reservation released the stashed slot"
    );
    let registry = fixture
        .recorder
        .registry(&parallel_key("api_key", "key-1"))
        .expect("the gauge holds a slot registry");
    assert!(!registry.contains_key("slot-1"));
}

fn assert_token_reservation_rejection(error: &PreCallHookError) {
    assert!(matches!(
        error,
        PreCallHookError::Reservation(TokenReservationError::RateLimited(_))
    ));
    let Some(PreCallHookRejection::RateLimited(rate_limited)) = error.rejection() else {
        panic!("the rejection is a 429");
    };
    assert!(
        rate_limited
            .message
            .starts_with("Rate limit exceeded for api_key: key-1")
    );
    assert_eq!(
        rate_limited.rate_limit_type,
        Some(RateLimitDimension::Tokens)
    );
}

#[tokio::test]
async fn hook_checks_the_tpm_in_the_first_pass_with_reservation_off() {
    let mut parts = Parts::new(
        entries([token_window("api_key", "key-1", 5)]),
        Vec::new(),
        None,
    );
    parts.reservation_off = true;
    let fixture = Fixture::new(&parts);
    let auth = support::auth(r#"{"api_key": "key-1", "tpm_limit": 5, "max_parallel_requests": 2}"#);
    let mut data = support::data(
        r#"{"model": "gpt4", "max_tokens": 10, "messages": [{"role": "user", "content": "hi"}]}"#,
    );
    let slot = RequestStashSlot::default();

    let Err(error) = fixture
        .run(Some(&slot), &auth, &mut data, CallType::Acompletion)
        .await
    else {
        panic!("the first pass checks the token counter");
    };

    assert!(matches!(&error, PreCallHookError::RateLimited(_)));
    assert!(fixture.recorder.gauge_writes().is_empty());
    assert!(snapshot(&slot).slot.is_none());
}

#[tokio::test]
async fn hook_fills_the_stash_after_a_successful_token_reservation() {
    let mut parts = Parts::new(Vec::new(), Vec::new(), None);
    parts.tags = vec![(String::from("t1"), None, Some(100))];
    let fixture = Fixture::new(&parts);
    let auth = support::auth(r#"{"api_key": "key-1", "tpm_limit": 100}"#);
    let mut data = support::data(
        r#"{"model": "gpt4", "max_tokens": 10, "messages": [{"role": "user", "content": "hi"}],
            "metadata": {"tags": ["t1"]}}"#,
    );
    let slot = RequestStashSlot::default();

    fixture
        .run(Some(&slot), &auth, &mut data, CallType::Acompletion)
        .await
        .expect("the reservation passes");

    let state = snapshot(&slot);
    assert_reserved_stash(&state);
    let response = state.response.expect("the first pass stored its response");
    assert_token_statuses(&response);
    assert_eq!(fixture.tags.calls(), [vec![String::from("t1")]]);
}

fn assert_reserved_stash(state: &StashSnapshot) {
    assert!(state.reserved_tokens > 0);
    assert_eq!(
        state.reserved_model,
        Some(RateLimitedModel {
            requested: String::from("gpt4"),
            group: String::from("gpt4"),
        })
    );
    assert_eq!(
        state.reserved_scopes,
        scopes(&[("api_key", "key-1"), ("tag", "t1")])
    );
    assert_eq!(state.tpm_limited_tags, BTreeSet::from([String::from("t1")]));
}

fn assert_token_statuses(response: &RateLimitResponse) {
    assert_eq!(response.statuses.len(), 2);
    assert!(
        response
            .statuses
            .iter()
            .all(|status| status.code == "OK" && status.rate_limit_type == RateLimitType::Tokens)
    );
}
