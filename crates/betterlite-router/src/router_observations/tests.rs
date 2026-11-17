#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_core::api::responses_usage::{Usage, UsageInput, normalize_usage};
use chrono::{DateTime, Utc};
use rstest::rstest;

use super::RouterObservations;
use super::counter::{CounterError, CounterSink, Increment};
use super::failure::{
    AsyncFailureEvent, CooldownError, CooldownSink, FailureError, FailureEvent, FailureObservation,
};
use super::keys::{ROUTER_COUNTER_TTL, outcome_keys, usage_keys};
use super::success::{ObservationMetadata, RequestFacts, ResponseEvent, SuccessEvent};
use super::usage::DeploymentUsage;
use crate::cooldown_decisions::failure::{
    ErrorStatus, FailureClass, FailureFacts, Seconds, TimeoutFacts,
};
use crate::retry_policy::retry_after::RetryAfter;

const NOW_MILLIS: i64 = 1_700_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Write {
    value: i64,
    ttl: Option<Duration>,
}

#[derive(Default)]
struct MemoryCounters {
    values: Mutex<BTreeMap<String, i64>>,
    writes: Mutex<BTreeMap<String, Vec<Write>>>,
    reject_increments: bool,
    reject_pipeline: bool,
}

impl MemoryCounters {
    fn rejecting_increments() -> Self {
        Self {
            reject_increments: true,
            ..Self::default()
        }
    }

    fn rejecting_pipeline() -> Self {
        Self {
            reject_pipeline: true,
            ..Self::default()
        }
    }

    fn value(&self, key: &str) -> Option<i64> {
        let values = self
            .values
            .lock()
            .expect("the counter fixture lock is not poisoned");
        values.get(key).copied()
    }

    fn writes(&self, key: &str) -> Vec<Write> {
        let writes = self
            .writes
            .lock()
            .expect("the write fixture lock is not poisoned");
        writes.get(key).cloned().unwrap_or_default()
    }

    fn record(&self, key: &str, value: i64, ttl: Option<Duration>) {
        let mut writes = self
            .writes
            .lock()
            .expect("the write fixture lock is not poisoned");
        writes
            .entry(key.to_owned())
            .or_default()
            .push(Write { value, ttl });
    }
}

impl CounterSink for MemoryCounters {
    fn increment(&self, key: &str, value: i64, ttl: Duration) -> Result<(), CounterError> {
        if self.reject_increments {
            return Err(CounterError::Rejected {
                key: key.to_owned(),
            });
        }
        let mut values = self
            .values
            .lock()
            .expect("the counter fixture lock is not poisoned");
        let counter = values.entry(key.to_owned()).or_insert(0);
        *counter = counter.saturating_add(value);
        drop(values);
        self.record(key, value, Some(ttl));
        Ok(())
    }

    fn increment_pipeline(&self, increments: &[Increment]) -> Result<(), CounterError> {
        if self.reject_pipeline {
            let key = increments
                .first()
                .map_or_else(String::new, |increment| increment.key.clone());
            return Err(CounterError::Rejected { key });
        }
        for increment in increments {
            self.increment(&increment.key, increment.value, increment.ttl)?;
        }
        Ok(())
    }

    fn read(&self, key: &str) -> Option<i64> {
        self.value(key)
    }

    fn write(&self, key: &str, value: i64, ttl: Option<Duration>) {
        let mut values = self
            .values
            .lock()
            .expect("the counter fixture lock is not poisoned");
        values.insert(key.to_owned(), value);
        drop(values);
        self.record(key, value, ttl);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Recorded {
    time_to_cooldown: Seconds,
    successes: u64,
    failures: u64,
}

#[derive(Default)]
struct MemoryCooldowns {
    recorded: Mutex<Vec<Recorded>>,
    answer: bool,
    reject: bool,
}

impl MemoryCooldowns {
    fn rejecting() -> Self {
        Self {
            reject: true,
            ..Self::default()
        }
    }

    fn last(&self) -> Option<Recorded> {
        let recorded = self
            .recorded
            .lock()
            .expect("the cooldown fixture lock is not poisoned");
        recorded.last().copied()
    }
}

impl CooldownSink for MemoryCooldowns {
    fn set_cooldown_deployments(
        &self,
        observation: &FailureObservation<'_>,
    ) -> Result<bool, CooldownError> {
        if self.reject {
            return Err(CooldownError::Rejected {
                deployment_id: observation.deployment_id.to_owned(),
            });
        }
        let mut recorded = self
            .recorded
            .lock()
            .expect("the cooldown fixture lock is not poisoned");
        recorded.push(Recorded {
            time_to_cooldown: observation.time_to_cooldown,
            successes: observation.minute.successes,
            failures: observation.minute.failures,
        });
        Ok(self.answer)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Evidence {
    key: Option<String>,
    successes: Option<i64>,
    failures: Option<i64>,
    tpm: Option<i64>,
    rpm: Option<i64>,
    request_count: Option<i64>,
    writes: Vec<Write>,
    cooldown: Option<bool>,
    time_to_cooldown: Option<Seconds>,
    minute: Option<(u64, u64)>,
    error: Option<String>,
}

#[derive(Default)]
struct Harness {
    counters: MemoryCounters,
    cooldowns: MemoryCooldowns,
}

impl Harness {
    fn observations(&self) -> RouterObservations<'_> {
        RouterObservations {
            counters: &self.counters,
            cooldowns: &self.cooldowns,
            cooldown_time: Seconds(5.0),
        }
    }

    fn evidence(&self, key: Option<String>) -> Evidence {
        let usage = usage_keys("dep-1", "openai/gpt-4o", now());
        let outcome = outcome_keys("dep-1");
        let recorded = self.cooldowns.last();
        Evidence {
            key,
            successes: self.counters.value(&outcome.successes),
            failures: self.counters.value(&outcome.failures),
            tpm: self.counters.value(&usage.tpm),
            rpm: self.counters.value(&usage.rpm),
            request_count: self.counters.value("dep-1"),
            time_to_cooldown: recorded.as_ref().map(|entry| entry.time_to_cooldown),
            minute: recorded
                .as_ref()
                .map(|entry| (entry.successes, entry.failures)),
            ..Evidence::default()
        }
    }
}

fn now() -> DateTime<Utc> {
    at(NOW_MILLIS)
}

fn at(unix_millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(unix_millis).expect("the fixture clock is in range")
}

fn request() -> RequestFacts<'static> {
    RequestFacts {
        model_group: Some("probe-group"),
        deployment_name: "openai/gpt-4o",
        deployment_id: Some("dep-1"),
    }
}

fn usage_limited() -> DeploymentUsage {
    DeploymentUsage {
        params_tpm: Some(1000),
        params_rpm: Some(100),
        ..DeploymentUsage::default()
    }
}

fn response_usage(total_tokens: i64) -> Usage {
    let mut usage = normalize_usage(UsageInput::Absent).expect("an absent usage normalizes");
    usage.total_tokens = total_tokens;
    usage
}

fn success_event(total_tokens: Option<i64>, deployment: &DeploymentUsage) -> SuccessEvent<'_> {
    SuccessEvent {
        call_type: None,
        total_tokens,
        request: request(),
        deployment: Some(deployment),
    }
}

fn failure(class: FailureClass, status: ErrorStatus) -> FailureFacts {
    FailureFacts {
        class,
        status,
        advisor_orchestration: false,
        background_response_cost_poll: false,
        wrapped_exception: None,
        caller_timeout: None,
    }
}

fn run_failure(
    harness: &Harness,
    facts: &FailureFacts,
    model_info: Option<Seconds>,
    params: Option<Seconds>,
    retry_after: Option<RetryAfter>,
) -> Evidence {
    let event = FailureEvent {
        deployment_id: Some("dep-1"),
        failure: facts,
        requested_model_group: Some("probe-group"),
        model_info_cooldown_time: model_info,
        litellm_params_cooldown_time: params,
        retry_after,
    };
    match harness.observations().on_failure(&event, NOW_MILLIS, now()) {
        Ok(cooldown) => Evidence {
            cooldown: Some(cooldown),
            ..harness.evidence(None)
        },
        Err(FailureError::Counter(CounterError::Rejected { key })) => Evidence {
            error: Some(format!("counter:{key}")),
            ..harness.evidence(None)
        },
        Err(FailureError::Cooldown(CooldownError::Rejected { deployment_id })) => Evidence {
            error: Some(format!("cooldown:{deployment_id}")),
            ..harness.evidence(None)
        },
    }
}

fn case_success() -> Evidence {
    let harness = Harness::default();
    let deployment = usage_limited();
    let key = harness.observations().on_success(
        &success_event(Some(100), &deployment),
        &ObservationMetadata::default(),
        now(),
    );
    harness.evidence(key)
}

fn case_success_without_limits() -> Evidence {
    let harness = Harness::default();
    let deployment = DeploymentUsage::default();
    let key = harness.observations().on_success(
        &success_event(Some(100), &deployment),
        &ObservationMetadata::default(),
        now(),
    );
    harness.evidence(key)
}

fn case_success_with_io_limits() -> Evidence {
    let harness = Harness::default();
    let deployment = DeploymentUsage {
        io_token_limits: true,
        ..DeploymentUsage::default()
    };
    let key = harness.observations().on_success(
        &success_event(Some(100), &deployment),
        &ObservationMetadata::default(),
        now(),
    );
    harness.evidence(key)
}

fn case_success_with_call_type(call_type: &'static str) -> Evidence {
    let harness = Harness::default();
    let deployment = usage_limited();
    let event = SuccessEvent {
        call_type: Some(call_type),
        ..success_event(Some(100), &deployment)
    };
    let key = harness
        .observations()
        .on_success(&event, &ObservationMetadata::default(), now());
    harness.evidence(key)
}

fn case_success_at(unix_millis: i64) -> Evidence {
    let harness = Harness::default();
    let deployment = usage_limited();
    let key = harness.observations().on_success(
        &success_event(Some(1), &deployment),
        &ObservationMetadata::default(),
        at(unix_millis),
    );
    harness.evidence(key)
}

fn case_success_without_logging_object() -> Evidence {
    let harness = Harness::default();
    let deployment = usage_limited();
    let key = harness.observations().on_success(
        &success_event(None, &deployment),
        &ObservationMetadata::default(),
        now(),
    );
    harness.evidence(key)
}

fn case_success_without_model_group() -> Evidence {
    let harness = Harness::default();
    let deployment = usage_limited();
    let event = SuccessEvent {
        request: RequestFacts {
            model_group: None,
            ..request()
        },
        ..success_event(Some(100), &deployment)
    };
    let key = harness
        .observations()
        .on_success(&event, &ObservationMetadata::default(), now());
    harness.evidence(key)
}

fn case_success_without_deployment_id() -> Evidence {
    let harness = Harness::default();
    let deployment = usage_limited();
    let event = SuccessEvent {
        request: RequestFacts {
            deployment_id: None,
            ..request()
        },
        ..success_event(Some(100), &deployment)
    };
    let key = harness
        .observations()
        .on_success(&event, &ObservationMetadata::default(), now());
    harness.evidence(key)
}

fn case_success_for_unknown_deployment() -> Evidence {
    let harness = Harness::default();
    let event = SuccessEvent {
        call_type: None,
        total_tokens: Some(100),
        request: request(),
        deployment: None,
    };
    let key = harness
        .observations()
        .on_success(&event, &ObservationMetadata::default(), now());
    harness.evidence(key)
}

fn case_response_then_success(response_total: i64, success_total: i64) -> Evidence {
    let harness = Harness::default();
    let deployment = usage_limited();
    let mut metadata = ObservationMetadata::default();
    let observations = harness.observations();
    let usage = response_usage(response_total);
    let response = ResponseEvent {
        usage: &usage,
        request: request(),
        deployment: Some(&deployment),
    };
    let _counted = observations.count_response_usage(&response, &mut metadata, now());
    let key = observations.on_success(
        &success_event(Some(success_total), &deployment),
        &metadata,
        now(),
    );
    harness.evidence(key)
}

fn case_response_without_a_deployment() -> Evidence {
    let harness = Harness::default();
    let mut metadata = ObservationMetadata::default();
    let usage = response_usage(25);
    let response = ResponseEvent {
        usage: &usage,
        request: request(),
        deployment: None,
    };
    let key = harness
        .observations()
        .count_response_usage(&response, &mut metadata, now());
    assert_eq!(metadata.counted_tokens, Some(25));
    harness.evidence(key)
}

fn case_response_without(facts: RequestFacts<'static>, total_tokens: i64) -> Evidence {
    let harness = Harness::default();
    let deployment = usage_limited();
    let mut metadata = ObservationMetadata::default();
    let usage = response_usage(total_tokens);
    let event = ResponseEvent {
        usage: &usage,
        request: facts,
        deployment: Some(&deployment),
    };
    let key = harness
        .observations()
        .count_response_usage(&event, &mut metadata, now());
    assert_eq!(metadata.counted_tokens, None);
    harness.evidence(key)
}

fn case_response_pipeline_failure() -> Evidence {
    let harness = Harness {
        counters: MemoryCounters::rejecting_pipeline(),
        cooldowns: MemoryCooldowns::default(),
    };
    let deployment = usage_limited();
    let mut metadata = ObservationMetadata::default();
    let usage = response_usage(25);
    let response = ResponseEvent {
        usage: &usage,
        request: request(),
        deployment: Some(&deployment),
    };
    let key = harness
        .observations()
        .count_response_usage(&response, &mut metadata, now());
    assert_eq!(metadata.counted_tokens, None);
    harness.evidence(key)
}

fn case_success_counter_failure() -> Evidence {
    let harness = Harness {
        counters: MemoryCounters::rejecting_increments(),
        cooldowns: MemoryCooldowns::default(),
    };
    let deployment = usage_limited();
    let key = harness.observations().on_success(
        &success_event(Some(100), &deployment),
        &ObservationMetadata::default(),
        now(),
    );
    harness.evidence(key)
}

fn case_success_pipeline_failure() -> Evidence {
    let harness = Harness {
        counters: MemoryCounters::rejecting_pipeline(),
        cooldowns: MemoryCooldowns::default(),
    };
    let deployment = usage_limited();
    let key = harness.observations().on_success(
        &success_event(Some(100), &deployment),
        &ObservationMetadata::default(),
        now(),
    );
    harness.evidence(key)
}

fn case_cancelled_request_keeps_the_usage_once() -> Evidence {
    let harness = Harness::default();
    let deployment = usage_limited();
    let observations = harness.observations();
    let usage = response_usage(60);
    let response = ResponseEvent {
        usage: &usage,
        request: request(),
        deployment: Some(&deployment),
    };
    let mut cancelled = ObservationMetadata::default();
    let _counted = observations.count_response_usage(&response, &mut cancelled, now());
    // the request is cancelled here: nobody calls the success callback, so the marker is never read again
    harness.evidence(None)
}

fn case_cleared_marker_counts_the_success_in_full() -> Evidence {
    let harness = Harness::default();
    let deployment = usage_limited();
    let observations = harness.observations();
    let usage = response_usage(60);
    let response = ResponseEvent {
        usage: &usage,
        request: request(),
        deployment: Some(&deployment),
    };
    let mut cleared = ObservationMetadata::default();
    let _counted = observations.count_response_usage(&response, &mut cleared, now());
    // the usage write failed, so the marker was cleared and a later success counts its tokens again
    cleared.counted_tokens = None;
    let key = observations.on_success(&success_event(Some(100), &deployment), &cleared, now());
    harness.evidence(key)
}

fn case_failure() -> Evidence {
    let harness = Harness::default();
    let facts = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run_failure(&harness, &facts, None, None, None)
}

fn case_failure_minute_counts() -> Evidence {
    let harness = Harness::default();
    harness.counters.write(
        &outcome_keys("dep-1").successes,
        3,
        Some(ROUTER_COUNTER_TTL),
    );
    let facts = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run_failure(&harness, &facts, None, None, None)
}

fn case_failure_filters(facts: &FailureFacts) -> Evidence {
    let harness = Harness::default();
    run_failure(&harness, facts, None, None, None)
}

fn case_failure_without_deployment() -> Evidence {
    let harness = Harness::default();
    let facts = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    let event = FailureEvent {
        deployment_id: None,
        failure: &facts,
        requested_model_group: Some("probe-group"),
        model_info_cooldown_time: None,
        litellm_params_cooldown_time: None,
        retry_after: None,
    };
    let mut evidence = harness.evidence(None);
    if let Ok(cooldown) = harness.observations().on_failure(&event, NOW_MILLIS, now()) {
        evidence.cooldown = Some(cooldown);
    }
    evidence
}

fn case_failure_counter_rejected() -> Evidence {
    let harness = Harness {
        counters: MemoryCounters::rejecting_increments(),
        cooldowns: MemoryCooldowns::default(),
    };
    let facts = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run_failure(&harness, &facts, None, None, None)
}

fn case_failure_cooldown_rejected() -> Evidence {
    let harness = Harness {
        counters: MemoryCounters::default(),
        cooldowns: MemoryCooldowns::rejecting(),
    };
    let facts = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run_failure(&harness, &facts, None, None, None)
}

fn case_failure_times(
    model_info: Option<Seconds>,
    params: Option<Seconds>,
    retry_after: Option<RetryAfter>,
) -> Evidence {
    let harness = Harness::default();
    let facts = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run_failure(&harness, &facts, model_info, params, retry_after)
}

fn case_async_failure(call_type: Option<&'static str>) -> Evidence {
    let harness = Harness::default();
    let event = AsyncFailureEvent {
        call_type,
        model_group: Some("probe-group"),
        deployment_name: "openai/gpt-4o",
        deployment_id: Some("dep-1"),
    };
    let result = harness.observations().on_async_failure(&event, now());
    let mut evidence = harness.evidence(None);
    if let Err(CounterError::Rejected { key }) = result {
        evidence.error = Some(format!("counter:{key}"));
    }
    evidence
}

fn case_async_failure_without(
    model_group: Option<&'static str>,
    deployment_id: Option<&'static str>,
) -> Evidence {
    let harness = Harness::default();
    let event = AsyncFailureEvent {
        call_type: None,
        model_group,
        deployment_name: "openai/gpt-4o",
        deployment_id,
    };
    let result = harness.observations().on_async_failure(&event, now());
    let mut evidence = harness.evidence(None);
    if let Err(CounterError::Rejected { key }) = result {
        evidence.error = Some(format!("counter:{key}"));
    }
    evidence
}

fn case_async_failure_twice() -> Evidence {
    let harness = Harness::default();
    let event = AsyncFailureEvent {
        call_type: None,
        model_group: Some("probe-group"),
        deployment_name: "openai/gpt-4o",
        deployment_id: Some("dep-1"),
    };
    let observations = harness.observations();
    let _first = observations.on_async_failure(&event, now());
    let _second = observations.on_async_failure(&event, now());
    harness.evidence(None)
}

fn case_update_usage_once() -> Evidence {
    let harness = Harness::default();
    let count = harness.observations().update_usage("dep-1");
    let mut evidence = harness.evidence(None);
    evidence.writes = harness.counters.writes("dep-1");
    evidence.request_count = Some(count);
    evidence
}

fn case_update_usage_twice() -> Evidence {
    let harness = Harness::default();
    let observations = harness.observations();
    let _first = observations.update_usage("dep-1");
    let count = observations.update_usage("dep-1");
    let mut evidence = harness.evidence(None);
    evidence.writes = harness.counters.writes("dep-1");
    evidence.request_count = Some(count);
    evidence
}

fn case_update_usage_saturates() -> Evidence {
    let harness = Harness::default();
    harness.counters.write("dep-1", i64::MAX, None);
    let count = harness.observations().update_usage("dep-1");
    let mut evidence = harness.evidence(None);
    evidence.request_count = Some(count);
    evidence
}

fn case_track(model_id: Option<&'static str>, response_present: bool) -> Evidence {
    let harness = Harness::default();
    harness
        .observations()
        .track_deployment_metrics(model_id, response_present);
    harness.evidence(None)
}

fn tpm_key(minute: &str) -> String {
    format!("global_router:dep-1:openai/gpt-4o:tpm:{minute}")
}

fn failures_key() -> String {
    outcome_keys("dep-1").failures
}

#[rstest]
#[case::success_counts_usage(
    case_success(),
    Evidence { key: Some(tpm_key("22-13")), successes: Some(1), tpm: Some(100), rpm: Some(1), ..Evidence::default() }
)]
#[case::success_without_limits_counts_only_the_success(
    case_success_without_limits(),
    Evidence { successes: Some(1), ..Evidence::default() }
)]
#[case::success_with_io_limits_only(case_success_with_io_limits(), Evidence {
    key: Some(tpm_key("22-13")),
    successes: Some(1),
    tpm: Some(100),
    rpm: Some(1),
    ..Evidence::default()
})]
#[case::response_then_success_counts_the_remainder(
    case_response_then_success(60, 100),
    Evidence { key: Some(tpm_key("22-13")), successes: Some(1), tpm: Some(100), rpm: Some(1), ..Evidence::default() }
)]
#[case::response_then_success_with_a_smaller_total(
    case_response_then_success(60, 40),
    Evidence { successes: Some(1), tpm: Some(60), rpm: Some(1), ..Evidence::default() }
)]
#[case::response_then_success_with_an_equal_total(
    case_response_then_success(60, 60),
    Evidence { successes: Some(1), tpm: Some(60), rpm: Some(1), ..Evidence::default() }
)]
#[case::response_then_success_after_zero_counted(
    case_response_then_success(0, 30),
    Evidence { key: Some(tpm_key("22-13")), successes: Some(1), tpm: Some(30), rpm: Some(1), ..Evidence::default() }
)]
#[case::response_without_a_known_deployment_still_marks(
    case_response_without_a_deployment(),
    Evidence::default()
)]
#[case::failure_counts_once_and_delegates(
    case_failure(),
    Evidence { failures: Some(1), cooldown: Some(false), time_to_cooldown: Some(Seconds(5.0)), minute: Some((0, 1)), ..Evidence::default() }
)]
#[case::failure_minute_carries_the_successes(
    case_failure_minute_counts(),
    Evidence { failures: Some(1), cooldown: Some(false), time_to_cooldown: Some(Seconds(5.0)), minute: Some((3, 1)), ..Evidence::default() }
)]
#[case::async_failure_counts_rpm_only(
    case_async_failure(None),
    Evidence { rpm: Some(1), ..Evidence::default() }
)]
#[case::async_failures_accumulate(
    case_async_failure_twice(),
    Evidence { rpm: Some(2), ..Evidence::default() }
)]
#[case::update_usage_stores_the_first_count(
    case_update_usage_once(),
    Evidence { request_count: Some(1), writes: vec![Write { value: 1, ttl: Some(Duration::from_secs(60)) }], ..Evidence::default() }
)]
#[case::update_usage_keeps_the_ttl(
    case_update_usage_twice(),
    Evidence { request_count: Some(2), writes: vec![Write { value: 1, ttl: Some(Duration::from_secs(60)) }, Write { value: 2, ttl: None }], ..Evidence::default() }
)]
#[case::track_without_a_response_counts_the_request(
    case_track(Some("dep-1"), false),
    Evidence { request_count: Some(1), ..Evidence::default() }
)]
#[case::track_with_a_response_counts_nothing(case_track(Some("dep-1"), true), Evidence::default())]
#[case::track_without_a_model_id_counts_nothing(case_track(None, false), Evidence::default())]
fn router_observations_behaviors(#[case] actual: Evidence, #[case] expected: Evidence) {
    assert_eq!(actual, expected);
}

#[rstest]
#[case::success_skips_a_websocket_session(
    case_success_with_call_type("_aresponses_websocket"),
    Evidence::default()
)]
#[case::success_skips_a_realtime_session(
    case_success_with_call_type("_arealtime"),
    Evidence::default()
)]
#[case::success_skips_a_batch_retrieve(
    case_success_with_call_type("retrieve_batch"),
    Evidence::default()
)]
#[case::success_skips_an_async_batch_retrieve(
    case_success_with_call_type("aretrieve_batch"),
    Evidence::default()
)]
#[case::success_counts_a_chat_call(case_success_with_call_type("completion"), Evidence {
    key: Some(tpm_key("22-13")),
    successes: Some(1),
    tpm: Some(100),
    rpm: Some(1),
    ..Evidence::default()
})]
#[case::success_without_a_logging_object_counts_nothing(
    case_success_without_logging_object(),
    Evidence::default()
)]
#[case::success_without_a_model_group_counts_nothing(
    case_success_without_model_group(),
    Evidence::default()
)]
#[case::success_without_a_deployment_id_counts_nothing(
    case_success_without_deployment_id(),
    Evidence::default()
)]
#[case::success_for_an_unknown_deployment_counts_nothing(
    case_success_for_unknown_deployment(),
    Evidence::default()
)]
#[case::success_counter_failure_stops_the_callback(
    case_success_counter_failure(),
    Evidence::default()
)]
#[case::response_usage_failure_clears_the_marker(
    case_response_pipeline_failure(),
    Evidence::default()
)]
#[case::cancelled_request_keeps_the_usage_once(
    case_cancelled_request_keeps_the_usage_once(),
    Evidence { tpm: Some(60), rpm: Some(1), ..Evidence::default() }
)]
#[case::a_cleared_marker_counts_the_success_in_full(
    case_cleared_marker_counts_the_success_in_full(),
    Evidence { key: Some(tpm_key("22-13")), successes: Some(1), tpm: Some(160), rpm: Some(2), ..Evidence::default() }
)]
#[case::failure_counter_failure_is_reported(
    case_failure_counter_rejected(),
    Evidence { error: Some(format!("counter:{}", failures_key())), ..Evidence::default() }
)]
#[case::failure_cooldown_failure_is_reported(
    case_failure_cooldown_rejected(),
    Evidence { failures: Some(1), error: Some("cooldown:dep-1".to_owned()), ..Evidence::default() }
)]
#[case::failure_without_a_deployment_counts_nothing(
    case_failure_without_deployment(),
    Evidence { cooldown: Some(false), ..Evidence::default() }
)]
#[case::advisor_orchestration_is_not_the_deployment(case_failure_filters(&FailureFacts {
    advisor_orchestration: true,
    ..failure(FailureClass::RateLimit, ErrorStatus::Code(429))
}), Evidence { cooldown: Some(false), ..Evidence::default() })]
#[case::background_cost_poll_404_is_not_the_deployment(case_failure_filters(&FailureFacts {
    background_response_cost_poll: true,
    ..failure(FailureClass::NotFound, ErrorStatus::Code(404))
}), Evidence { cooldown: Some(false), ..Evidence::default() })]
#[case::a_background_cost_poll_500_is_the_deployment(case_failure_filters(&FailureFacts {
    background_response_cost_poll: true,
    ..failure(FailureClass::InternalServer, ErrorStatus::Code(500))
}), Evidence { failures: Some(1), cooldown: Some(false), minute: Some((0, 1)), ..Evidence::default() })]
#[case::a_plain_404_is_the_deployment(case_failure_filters(&failure(FailureClass::NotFound, ErrorStatus::Code(404))), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::a_caller_timeout_at_the_limit_is_not_the_deployment(case_failure_filters(&FailureFacts {
    caller_timeout: Some(TimeoutFacts { client_side_timeout: true, timeout_seconds: Some(Seconds(5.0)), elapsed_seconds: Some(Seconds(5.0)) }),
    ..failure(FailureClass::Timeout, ErrorStatus::Code(408))
}), Evidence { cooldown: Some(false), ..Evidence::default() })]
#[case::a_caller_timeout_under_the_limit_is_the_deployment(case_failure_filters(&FailureFacts {
    caller_timeout: Some(TimeoutFacts { client_side_timeout: true, timeout_seconds: Some(Seconds(5.0)), elapsed_seconds: Some(Seconds(4.0)) }),
    ..failure(FailureClass::Timeout, ErrorStatus::Code(408))
}), Evidence { failures: Some(1), cooldown: Some(false), minute: Some((0, 1)), ..Evidence::default() })]
#[case::a_provider_timeout_is_the_deployment(case_failure_filters(&FailureFacts {
    caller_timeout: Some(TimeoutFacts { client_side_timeout: false, timeout_seconds: Some(Seconds(5.0)), elapsed_seconds: Some(Seconds(9.0)) }),
    ..failure(FailureClass::Timeout, ErrorStatus::Code(408))
}), Evidence { failures: Some(1), cooldown: Some(false), minute: Some((0, 1)), ..Evidence::default() })]
#[case::model_info_cooldown_time_wins(case_failure_times(Some(Seconds(1.5)), Some(Seconds(2.5)), Some(RetryAfter::Seconds(7))), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(1.5)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::litellm_params_cooldown_time_applies(case_failure_times(None, Some(Seconds(2.5)), Some(RetryAfter::Seconds(7))), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(2.5)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::a_negative_model_info_value_falls_to_the_header(case_failure_times(Some(Seconds(-1.0)), Some(Seconds(2.5)), Some(RetryAfter::Seconds(7))), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(7.0)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::a_negative_litellm_params_value_falls_to_the_header(case_failure_times(None, Some(Seconds(-1.0)), Some(RetryAfter::Seconds(4))), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(4.0)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::a_negative_header_falls_to_the_router_default(case_failure_times(None, None, Some(RetryAfter::Seconds(-1))), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(5.0)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::a_header_date_counts_from_the_probe_clock(case_failure_times(None, None, Some(RetryAfter::At { unix_millis: 1_700_000_003_500 })), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(3.0)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::a_zero_cooldown_time_is_kept(case_failure_times(Some(Seconds(0.0)), None, Some(RetryAfter::Seconds(7))), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(0.0)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::last_minute_of_the_hour(case_success_at(1_700_006_399_999), Evidence {
    key: Some(tpm_key("23-59")),
    successes: Some(1),
    ..Evidence::default()
})]
#[case::first_minute_of_the_hour(case_success_at(1_700_006_400_000), Evidence {
    key: Some(tpm_key("00-00")),
    successes: Some(1),
    ..Evidence::default()
})]
#[case::single_digit_hour_is_padded(case_success_at(1_699_952_700_000), Evidence {
    key: Some(tpm_key("09-05")),
    successes: Some(1),
    ..Evidence::default()
})]
#[case::update_usage_saturates(case_update_usage_saturates(), Evidence {
    request_count: Some(i64::MAX),
    ..Evidence::default()
})]
#[case::async_failure_skips_a_batch_retrieve(
    case_async_failure(Some("retrieve_batch")),
    Evidence::default()
)]
#[case::async_failure_skips_an_async_batch_retrieve(
    case_async_failure(Some("aretrieve_batch")),
    Evidence::default()
)]
#[case::async_failure_without_a_model_group_counts_nothing(
    case_async_failure_without(None, Some("dep-1")),
    Evidence::default()
)]
#[case::async_failure_without_a_deployment_id_counts_nothing(
    case_async_failure_without(Some("probe-group"), None),
    Evidence::default()
)]
#[case::response_without_a_model_group_marks_nothing(
    case_response_without(RequestFacts { model_group: None, ..request() }, 25),
    Evidence::default()
)]
#[case::response_without_a_deployment_id_marks_nothing(
    case_response_without(RequestFacts { deployment_id: None, ..request() }, 25),
    Evidence::default()
)]
#[case::success_usage_failure_keeps_the_success_count(
    case_success_pipeline_failure(),
    Evidence { successes: Some(1), ..Evidence::default() }
)]
fn router_observations_boundaries(#[case] actual: Evidence, #[case] expected: Evidence) {
    assert_eq!(actual, expected);
}
