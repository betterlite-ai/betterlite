#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::api::request_destination::RequestData;
use betterlite_core::routes::CallType;
use betterlite_models::decisions::Json;

use super::{ModelGroupInfoError, PreCallError};
use crate::dynamic_admission::admission::AdmissionKey;
use crate::dynamic_admission::tests::support::{
    AdmissionSetup, Canned, GroupAnswer, Harness, MODEL, capture, counter_key, key, limits,
    metadata, ok, over_limit, percent, policy, request, response, settlement_failure, stash_owner,
    stored_response, unverifiable_failure, with_call_id,
};
use crate::dynamic_priority::descriptors::{PRIORITY_DESCRIPTOR_KEY, SATURATION_DESCRIPTOR_KEY};
use crate::dynamic_priority::settings::PriorityPolicy;
use crate::limiter_records::{CacheCounterValue, RateLimitType};
use crate::request_stash::RequestStashSlot;

fn reserved_policy() -> PriorityPolicy {
    policy(0.5, Some(&[("default", percent(0.5))]))
}

fn setup(
    router: Option<GroupAnswer>,
    seeded: Vec<(String, CacheCounterValue)>,
    atomic: Canned,
    tracking: Canned,
) -> AdmissionSetup {
    AdmissionSetup {
        policy: reserved_policy(),
        router,
        seeded,
        atomic,
        tracking,
    }
}

fn group_present() -> Option<GroupAnswer> {
    Some(GroupAnswer::Limits(limits(Some(100), Some(1000))))
}

fn allowed_atomic() -> Canned {
    Ok(response(
        "OK",
        vec![ok(SATURATION_DESCRIPTOR_KEY, RateLimitType::Requests)],
    ))
}

fn empty_tracking() -> Canned {
    Ok(response("OK", Vec::new()))
}

fn seeded_requests(count: i64) -> Vec<(String, CacheCounterValue)> {
    vec![(
        counter_key(MODEL, RateLimitType::Requests),
        CacheCounterValue::Int(count),
    )]
}

fn data_with_model(model: Json) -> RequestData {
    let mut data = RequestData::new();
    data.insert(String::from("model"), model);
    data
}

fn run_pre_call(
    harness: &Harness,
    data: &RequestData,
    admission_key: &AdmissionKey<'_>,
    stash: &RequestStashSlot,
) -> (Result<(), PreCallError>, String) {
    let mut outcome = None;
    let logs = capture(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("the test runtime starts");
        outcome = Some(runtime.block_on(harness.admission.pre_call(
            admission_key,
            data,
            CallType::Acompletion,
            stash,
        )));
    });
    (outcome.expect("the hook ran"), logs)
}

#[test]
fn hook_without_a_model_returns_before_touching_anything() {
    let harness = Harness::build(setup(
        group_present(),
        Vec::new(),
        allowed_atomic(),
        empty_tracking(),
    ));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, _logs) = run_pre_call(&harness, &RequestData::new(), &admission_key, &stash);
    outcome.expect("a request without a model is allowed");
    assert!(stash.get().is_none());
    assert!(harness.router().calls().is_empty());
    assert!(harness.limiter.atomic_calls().is_empty());
    assert!(harness.cache.reads().is_empty());
}

#[test]
fn hook_with_a_null_model_returns_before_touching_anything() {
    let harness = Harness::build(setup(
        group_present(),
        Vec::new(),
        allowed_atomic(),
        empty_tracking(),
    ));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, _logs) = run_pre_call(
        &harness,
        &data_with_model(Json::Null),
        &admission_key,
        &stash,
    );
    outcome.expect("a null model is no model");
    assert!(stash.get().is_none());
    assert!(harness.router().calls().is_empty());
    assert!(harness.limiter.atomic_calls().is_empty());
}

#[test]
fn hook_with_a_non_text_model_returns_before_touching_anything() {
    let harness = Harness::build(setup(
        group_present(),
        Vec::new(),
        allowed_atomic(),
        empty_tracking(),
    ));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, _logs) = run_pre_call(
        &harness,
        &data_with_model(Json::Bool(true)),
        &admission_key,
        &stash,
    );
    outcome.expect("a model that is not text names no group");
    assert!(stash.get().is_none());
    assert!(harness.router().calls().is_empty());
}

#[test]
fn hook_claims_the_stash_for_the_call_id() {
    let harness = Harness::build(setup(
        Some(GroupAnswer::Missing),
        Vec::new(),
        allowed_atomic(),
        empty_tracking(),
    ));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, _logs) = run_pre_call(
        &harness,
        &with_call_id(request(MODEL), "call-1"),
        &admission_key,
        &stash,
    );
    outcome.expect("a model without group info is allowed");
    assert_eq!(stash_owner(&stash), Some(String::from("call-1")));
}

#[test]
fn hook_leaves_the_owner_alone_for_a_non_text_call_id() {
    let harness = Harness::build(setup(
        Some(GroupAnswer::Missing),
        Vec::new(),
        allowed_atomic(),
        empty_tracking(),
    ));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let mut data = request(MODEL);
    data.insert(String::from("litellm_call_id"), Json::Bool(true));
    let (outcome, _logs) = run_pre_call(&harness, &data, &admission_key, &stash);
    outcome.expect("a model without group info is allowed");
    assert!(stash.get().is_some());
    assert_eq!(stash_owner(&stash), None);
}

#[test]
fn hook_without_a_router_returns_the_router_error() {
    let harness = Harness::build(setup(None, Vec::new(), allowed_atomic(), empty_tracking()));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, _logs) = run_pre_call(
        &harness,
        &with_call_id(request(MODEL), "call-1"),
        &admission_key,
        &stash,
    );
    let failure = outcome.expect_err("there is no router to read the group from");
    assert!(matches!(
        failure,
        PreCallError::ModelGroupInfo(ModelGroupInfoError::NoRouter { model }) if model == MODEL
    ));
    assert!(harness.limiter.atomic_calls().is_empty());
    assert!(harness.cache.reads().is_empty());
    assert_eq!(stash_owner(&stash), Some(String::from("call-1")));
}

#[test]
fn hook_without_group_info_allows_the_request() {
    let harness = Harness::build(setup(
        Some(GroupAnswer::Missing),
        Vec::new(),
        allowed_atomic(),
        empty_tracking(),
    ));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, logs) = run_pre_call(&harness, &request(MODEL), &admission_key, &stash);
    outcome.expect("a model without group info is allowed");
    assert!(
        logs.contains("No model group info for gpt-4, allowing request"),
        "logs: {logs}"
    );
    assert!(harness.cache.reads().is_empty());
    assert!(harness.limiter.atomic_calls().is_empty());
    assert_eq!(harness.router().calls(), vec![String::from(MODEL)]);
}

#[test]
fn hook_router_failure_is_not_fail_open() {
    let harness = Harness::build(setup(
        Some(GroupAnswer::Unavailable),
        Vec::new(),
        allowed_atomic(),
        empty_tracking(),
    ));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, _logs) = run_pre_call(&harness, &request(MODEL), &admission_key, &stash);
    let failure = outcome.expect_err("the router failure leaves the hook uncaught");
    assert!(matches!(
        failure,
        PreCallError::ModelGroupInfo(ModelGroupInfoError::Unavailable { model }) if model == MODEL
    ));
    assert!(harness.cache.reads().is_empty());
    assert!(harness.limiter.atomic_calls().is_empty());
}

#[test]
fn hook_below_threshold_logs_the_saturation_and_tracks() {
    let harness = Harness::build(setup(
        group_present(),
        seeded_requests(30),
        allowed_atomic(),
        empty_tracking(),
    ));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, logs) = run_pre_call(&harness, &request(MODEL), &admission_key, &stash);
    outcome.expect("the request is allowed");
    assert!(
        logs.contains(
            "[Dynamic Rate Limiter] Model=gpt-4, Saturation=30.0%, Threshold=50.0%, Priority=default"
        ),
        "logs: {logs}"
    );
    assert_eq!(harness.limiter.tracking_calls().len(), 1);
    assert_eq!(
        harness.limiter.tracking_calls()[0].descriptors[0].key,
        PRIORITY_DESCRIPTOR_KEY
    );
    let stored = stored_response(&stash).expect("the answer is stored");
    assert_eq!(stored.overall_code, "OK");
}

#[test]
fn hook_logs_none_for_an_absent_priority() {
    let harness = Harness::build(setup(
        group_present(),
        seeded_requests(30),
        allowed_atomic(),
        empty_tracking(),
    ));
    let key_metadata = BTreeMap::new();
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, logs) = run_pre_call(&harness, &request(MODEL), &admission_key, &stash);
    outcome.expect("the request is allowed");
    assert!(
        logs.contains(
            "[Dynamic Rate Limiter] Model=gpt-4, Saturation=30.0%, Threshold=50.0%, Priority=None"
        ),
        "logs: {logs}"
    );
}

#[test]
fn hook_saturated_call_enforces_the_priority_pool() {
    let harness = Harness::build(setup(
        group_present(),
        seeded_requests(60),
        allowed_atomic(),
        empty_tracking(),
    ));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, logs) = run_pre_call(&harness, &request(MODEL), &admission_key, &stash);
    outcome.expect("the request is allowed");
    assert!(
        logs.contains(
            "[Dynamic Rate Limiter] Model=gpt-4, Saturation=60.0%, Threshold=50.0%, Priority=default"
        ),
        "logs: {logs}"
    );
    assert_eq!(harness.limiter.atomic_calls()[0].descriptors.len(), 2);
    assert!(harness.limiter.tracking_calls().is_empty());
}

#[test]
fn hook_unreadable_counter_allows_and_tracks() {
    let harness = Harness::build(setup(
        group_present(),
        vec![(
            counter_key(MODEL, RateLimitType::Requests),
            CacheCounterValue::Text(String::from("not-a-number")),
        )],
        allowed_atomic(),
        empty_tracking(),
    ));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, logs) = run_pre_call(&harness, &request(MODEL), &admission_key, &stash);
    outcome.expect("an unreadable counter reads as no saturation");
    assert!(
        logs.contains("Error checking saturation for gpt-4"),
        "logs: {logs}"
    );
    assert!(logs.contains("Saturation=0.0%"), "logs: {logs}");
    assert_eq!(harness.limiter.tracking_calls().len(), 1);
}

#[test]
fn hook_rejection_reaches_the_caller() {
    let harness = Harness::build(setup(
        group_present(),
        seeded_requests(60),
        Ok(response(
            "OVER_LIMIT",
            vec![over_limit(
                SATURATION_DESCRIPTOR_KEY,
                RateLimitType::Requests,
            )],
        )),
        empty_tracking(),
    ));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, _logs) = run_pre_call(&harness, &request(MODEL), &admission_key, &stash);
    let failure = outcome.expect_err("the model-wide counter overflowed");
    let rejection = match failure {
        PreCallError::Rejection(error) => error,
        other => panic!("the known status names a 429, got {other:?}"),
    };
    assert!(
        rejection
            .message
            .starts_with("Model capacity reached for gpt-4."),
        "message: {}",
        rejection.message
    );
    assert!(stored_response(&stash).is_none());
}

#[test]
fn hook_fail_closed_503_reaches_the_caller() {
    let harness = Harness::build(setup(
        group_present(),
        seeded_requests(30),
        Err(unverifiable_failure()),
        empty_tracking(),
    ));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, logs) = run_pre_call(&harness, &request(MODEL), &admission_key, &stash);
    let failure = outcome.expect_err("the fail-closed 503 is re-raised");
    match failure {
        PreCallError::Unverifiable(error) => assert_eq!(error.status_code, 503),
        other => panic!("the unverifiable error keeps its variant, got {other:?}"),
    }
    assert!(
        !logs.contains("Error in dynamic rate limiter"),
        "logs: {logs}"
    );
}

#[test]
fn hook_other_limiter_errors_are_logged_and_allowed() {
    let harness = Harness::build(setup(
        group_present(),
        seeded_requests(30),
        Err(settlement_failure()),
        empty_tracking(),
    ));
    let key_metadata = metadata("default");
    let admission_key = key(&key_metadata);
    let stash = RequestStashSlot::default();
    let (outcome, logs) = run_pre_call(&harness, &request(MODEL), &admission_key, &stash);
    outcome.expect("a script failure never blocks the request");
    assert!(
        logs.contains("Error in dynamic rate limiter:"),
        "logs: {logs}"
    );
    assert!(logs.contains("allowing request"), "logs: {logs}");
    assert!(stored_response(&stash).is_none());
}
