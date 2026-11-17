#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::responses_usage::{Usage, UsageInput, normalize_usage};
use chrono::{DateTime, Utc};
use rstest::rstest;

use super::RouterObservations;
use super::counter::{CounterError, CounterSink, Increment};
use super::failure::{
    AsyncFailureEvent, CooldownError, CooldownSink, FailureError, FailureEvent, FailureObservation,
};
use super::success::{ObservationMetadata, RequestFacts, ResponseEvent, SuccessEvent};
use super::usage::DeploymentUsage;
use super::{failure, success, track};
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
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
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
        self.values
            .lock()
            .expect("the counter fixture lock is not poisoned")
            .insert(key.to_owned(), value);
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
        let mut recorded = self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        recorded.push(Recorded {
            time_to_cooldown: observation.time_to_cooldown,
            successes: observation.minute.successes,
            failures: observation.minute.failures,
        });
        drop(recorded);
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

#[derive(Clone, Copy)]
enum Rejection {
    Increments,
    Pipeline,
    Cooldowns,
}

#[derive(Default)]
struct Harness {
    counters: MemoryCounters,
    cooldowns: MemoryCooldowns,
}

impl Harness {
    fn rejecting(rejection: Rejection) -> Self {
        Self {
            counters: MemoryCounters {
                values: Mutex::new(BTreeMap::new()),
                writes: Mutex::new(BTreeMap::new()),
                reject_increments: matches!(rejection, Rejection::Increments),
                reject_pipeline: matches!(rejection, Rejection::Pipeline),
            },
            cooldowns: MemoryCooldowns {
                recorded: Mutex::new(Vec::new()),
                answer: false,
                reject: matches!(rejection, Rejection::Cooldowns),
            },
        }
    }

    fn observations(&self) -> RouterObservations<'_> {
        RouterObservations {
            counters: &self.counters,
            cooldowns: &self.cooldowns,
            cooldown_time: Seconds(5.0),
        }
    }

    fn evidence(&self, key: Option<String>) -> Evidence {
        let recorded = self.cooldowns.last();
        Evidence {
            key,
            successes: self.counters.value("dep-1:successes"),
            failures: self.counters.value("dep-1:fails"),
            tpm: self
                .counters
                .value("global_router:dep-1:openai/gpt-4o:tpm:22-13"),
            rpm: self
                .counters
                .value("global_router:dep-1:openai/gpt-4o:rpm:22-13"),
            request_count: self.counters.value("dep-1"),
            writes: Vec::new(),
            cooldown: None,
            time_to_cooldown: recorded.map(|entry| entry.time_to_cooldown),
            minute: recorded.map(|entry| (entry.successes, entry.failures)),
            error: None,
        }
    }
}

fn assert_counter_ttl(harness: &Harness, key: &str) {
    if let Some(recorded) = harness.counters.writes(key).first() {
        assert_eq!(recorded.ttl, Some(Duration::from_secs(60)));
    }
}

fn assert_usage_ttls(harness: &Harness) {
    assert_counter_ttl(harness, &tpm_key("22-13"));
    assert_counter_ttl(harness, "global_router:dep-1:openai/gpt-4o:rpm:22-13");
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
        tpm: None,
        rpm: None,
        params_tpm: Some(1000),
        params_rpm: Some(100),
        model_info_tpm: None,
        model_info_rpm: None,
        io_token_limits: false,
    }
}

fn response_usage(total_tokens: i64) -> Usage {
    let mut usage = normalize_usage(UsageInput::Absent).expect("an absent usage normalizes");
    usage.total_tokens = total_tokens;
    usage
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

fn tpm_key(minute: &str) -> String {
    format!("global_router:dep-1:openai/gpt-4o:tpm:{minute}")
}

fn case_success(harness: &Harness, event: &SuccessEvent<'_>, unix_millis: i64) -> Evidence {
    let observations = harness.observations();
    let key = success::on_success(
        &observations,
        event,
        &ObservationMetadata::default(),
        at(unix_millis),
    );
    assert_counter_ttl(harness, "dep-1:successes");
    assert_usage_ttls(harness);
    harness.evidence(key)
}

fn case_response(
    harness: &Harness,
    request_facts: RequestFacts<'static>,
    deployment: Option<&DeploymentUsage>,
    total_tokens: i64,
    counted: Option<i64>,
) -> Evidence {
    let usage = response_usage(total_tokens);
    let event = ResponseEvent {
        usage: &usage,
        request: request_facts,
        deployment,
    };
    let mut metadata = ObservationMetadata::default();
    let observations = harness.observations();
    let key = success::count_response_usage(&observations, &event, &mut metadata, now());
    assert_eq!(metadata.counted_tokens, counted);
    assert_usage_ttls(harness);
    harness.evidence(key)
}

fn case_response_then_success(
    responded: i64,
    total_tokens: Option<i64>,
    clear_marker: bool,
) -> Evidence {
    let harness = Harness::default();
    let deployment = usage_limited();
    let usage = response_usage(responded);
    let event = ResponseEvent {
        usage: &usage,
        request: request(),
        deployment: Some(&deployment),
    };
    let observations = harness.observations();
    let mut metadata = ObservationMetadata::default();
    let _counted = success::count_response_usage(&observations, &event, &mut metadata, now());
    if clear_marker {
        metadata.counted_tokens = None;
    }
    let key = total_tokens.and_then(|total| {
        success::on_success(
            &observations,
            &SuccessEvent {
                call_type: None,
                total_tokens: Some(total),
                request: request(),
                deployment: Some(&deployment),
            },
            &metadata,
            now(),
        )
    });
    harness.evidence(key)
}

fn case_failure(
    harness: &Harness,
    facts: &FailureFacts,
    cooldown_times: (Option<Seconds>, Option<Seconds>, Option<RetryAfter>),
    deployment_id: Option<&'static str>,
    successes: Option<i64>,
) -> Evidence {
    if let Some(count) = successes {
        harness
            .counters
            .write("dep-1:successes", count, Some(Duration::from_secs(60)));
    }
    let event = FailureEvent {
        deployment_id,
        failure: facts,
        requested_model_group: Some("probe-group"),
        model_info_cooldown_time: cooldown_times.0,
        litellm_params_cooldown_time: cooldown_times.1,
        retry_after: cooldown_times.2,
    };
    let outcome = failure::on_failure(&harness.observations(), &event, NOW_MILLIS);
    let mut evidence = harness.evidence(None);
    match outcome {
        Ok(cooldown) => evidence.cooldown = Some(cooldown),
        Err(FailureError::Counter(CounterError::Rejected { key })) => {
            evidence.error = Some(format!("counter:{key}"));
        }
        Err(FailureError::Cooldown(CooldownError::Rejected { deployment_id: id })) => {
            evidence.error = Some(format!("cooldown:{id}"));
        }
    }
    assert_counter_ttl(harness, "dep-1:fails");
    evidence
}

fn case_async_failure(harness: &Harness, event: &AsyncFailureEvent<'_>, calls: u32) -> Evidence {
    let observations = harness.observations();
    let mut result: Result<(), CounterError> = Ok(());
    for _ in 0..calls {
        result = failure::on_async_failure(&observations, event, now());
    }
    let mut evidence = harness.evidence(None);
    if let Err(CounterError::Rejected { key }) = result {
        evidence.error = Some(format!("counter:{key}"));
    }
    assert_counter_ttl(harness, "global_router:dep-1:openai/gpt-4o:rpm:22-13");
    evidence
}

fn case_update_usage(harness: &Harness, count_before: Option<i64>, calls: u32) -> Evidence {
    if let Some(count) = count_before {
        harness.counters.write("dep-1", count, None);
    }
    let observations = harness.observations();
    let mut count = 0;
    for _ in 0..calls {
        count = track::update_usage(&observations, "dep-1");
    }
    let mut evidence = harness.evidence(None);
    evidence.writes = harness.counters.writes("dep-1");
    evidence.request_count = Some(count);
    evidence
}

fn case_track(model_id: Option<&'static str>, response_present: bool) -> Evidence {
    let harness = Harness::default();
    track::track_deployment_metrics(&harness.observations(), model_id, response_present);
    harness.evidence(None)
}

#[rstest]
#[case::success_counts_usage(
    case_success(&Harness::default(), &SuccessEvent {
        call_type: None,
        total_tokens: Some(100),
        request: request(),
        deployment: Some(&usage_limited()),
    }, NOW_MILLIS),
    Evidence { key: Some(tpm_key("22-13")), successes: Some(1), tpm: Some(100), rpm: Some(1), ..Evidence::default() }
)]
#[case::success_without_limits_counts_only_the_success(
    case_success(&Harness::default(), &SuccessEvent {
        call_type: None,
        total_tokens: Some(100),
        request: request(),
        deployment: Some(&DeploymentUsage::default()),
    }, NOW_MILLIS),
    Evidence { successes: Some(1), ..Evidence::default() }
)]
#[case::success_with_io_limits_only(case_success(&Harness::default(), &SuccessEvent {
    call_type: None,
    total_tokens: Some(100),
    request: request(),
    deployment: Some(&DeploymentUsage {
        tpm: None,
        rpm: None,
        params_tpm: None,
        params_rpm: None,
        model_info_tpm: None,
        model_info_rpm: None,
        io_token_limits: true,
    }),
}, NOW_MILLIS), Evidence {
    key: Some(tpm_key("22-13")),
    successes: Some(1),
    tpm: Some(100),
    rpm: Some(1),
    ..Evidence::default()
})]
#[case::response_then_success_counts_the_remainder(
    case_response_then_success(60, Some(100), false),
    Evidence { key: Some(tpm_key("22-13")), successes: Some(1), tpm: Some(100), rpm: Some(1), ..Evidence::default() }
)]
#[case::response_then_success_with_a_smaller_total(
    case_response_then_success(60, Some(40), false),
    Evidence { successes: Some(1), tpm: Some(60), rpm: Some(1), ..Evidence::default() }
)]
#[case::response_then_success_with_an_equal_total(
    case_response_then_success(60, Some(60), false),
    Evidence { successes: Some(1), tpm: Some(60), rpm: Some(1), ..Evidence::default() }
)]
#[case::response_then_success_after_zero_counted(
    case_response_then_success(0, Some(30), false),
    Evidence { key: Some(tpm_key("22-13")), successes: Some(1), tpm: Some(30), rpm: Some(1), ..Evidence::default() }
)]
#[case::response_without_a_known_deployment_still_marks(
    case_response(&Harness::default(), request(), None, 25, Some(25)),
    Evidence::default()
)]
#[case::failure_counts_once_and_delegates(
    case_failure(&Harness::default(), &failure(FailureClass::RateLimit, ErrorStatus::Code(429)), (None, None, None), Some("dep-1"), None),
    Evidence { failures: Some(1), cooldown: Some(false), time_to_cooldown: Some(Seconds(5.0)), minute: Some((0, 1)), ..Evidence::default() }
)]
#[case::failure_minute_carries_the_successes(
    case_failure(&Harness::default(), &failure(FailureClass::RateLimit, ErrorStatus::Code(429)), (None, None, None), Some("dep-1"), Some(3)),
    Evidence { successes: Some(3), failures: Some(1), cooldown: Some(false), time_to_cooldown: Some(Seconds(5.0)), minute: Some((3, 1)), ..Evidence::default() }
)]
#[case::async_failure_counts_rpm_only(
    case_async_failure(&Harness::default(), &AsyncFailureEvent {
        call_type: None,
        model_group: Some("probe-group"),
        deployment_name: "openai/gpt-4o",
        deployment_id: Some("dep-1"),
    }, 1),
    Evidence { rpm: Some(1), ..Evidence::default() }
)]
#[case::async_failures_accumulate(
    case_async_failure(&Harness::default(), &AsyncFailureEvent {
        call_type: None,
        model_group: Some("probe-group"),
        deployment_name: "openai/gpt-4o",
        deployment_id: Some("dep-1"),
    }, 2),
    Evidence { rpm: Some(2), ..Evidence::default() }
)]
#[case::update_usage_stores_the_first_count(
    case_update_usage(&Harness::default(), None, 1),
    Evidence { request_count: Some(1), writes: vec![Write { value: 1, ttl: Some(Duration::from_secs(60)) }], ..Evidence::default() }
)]
#[case::update_usage_keeps_the_ttl(
    case_update_usage(&Harness::default(), None, 2),
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
    case_success(&Harness::default(), &SuccessEvent {
        call_type: Some("_aresponses_websocket"),
        total_tokens: Some(100),
        request: request(),
        deployment: Some(&usage_limited()),
    }, NOW_MILLIS),
    Evidence::default()
)]
#[case::success_skips_a_realtime_session(
    case_success(&Harness::default(), &SuccessEvent {
        call_type: Some("_arealtime"),
        total_tokens: Some(100),
        request: request(),
        deployment: Some(&usage_limited()),
    }, NOW_MILLIS),
    Evidence::default()
)]
#[case::success_skips_a_batch_retrieve(
    case_success(&Harness::default(), &SuccessEvent {
        call_type: Some("retrieve_batch"),
        total_tokens: Some(100),
        request: request(),
        deployment: Some(&usage_limited()),
    }, NOW_MILLIS),
    Evidence::default()
)]
#[case::success_skips_an_async_batch_retrieve(
    case_success(&Harness::default(), &SuccessEvent {
        call_type: Some("aretrieve_batch"),
        total_tokens: Some(100),
        request: request(),
        deployment: Some(&usage_limited()),
    }, NOW_MILLIS),
    Evidence::default()
)]
#[case::success_counts_a_chat_call(case_success(&Harness::default(), &SuccessEvent {
    call_type: Some("completion"),
    total_tokens: Some(100),
    request: request(),
    deployment: Some(&usage_limited()),
}, NOW_MILLIS), Evidence {
    key: Some(tpm_key("22-13")),
    successes: Some(1),
    tpm: Some(100),
    rpm: Some(1),
    ..Evidence::default()
})]
#[case::success_without_a_logging_object_counts_nothing(
    case_success(&Harness::default(), &SuccessEvent {
        call_type: None,
        total_tokens: None,
        request: request(),
        deployment: Some(&usage_limited()),
    }, NOW_MILLIS),
    Evidence::default()
)]
#[case::success_without_a_model_group_counts_nothing(
    case_success(&Harness::default(), &SuccessEvent {
        call_type: None,
        total_tokens: Some(100),
        request: RequestFacts { model_group: None, ..request() },
        deployment: Some(&usage_limited()),
    }, NOW_MILLIS),
    Evidence::default()
)]
#[case::success_without_a_deployment_id_counts_nothing(
    case_success(&Harness::default(), &SuccessEvent {
        call_type: None,
        total_tokens: Some(100),
        request: RequestFacts { deployment_id: None, ..request() },
        deployment: Some(&usage_limited()),
    }, NOW_MILLIS),
    Evidence::default()
)]
#[case::success_for_an_unknown_deployment_counts_nothing(
    case_success(&Harness::default(), &SuccessEvent {
        call_type: None,
        total_tokens: Some(100),
        request: request(),
        deployment: None,
    }, NOW_MILLIS),
    Evidence::default()
)]
#[case::success_counter_failure_stops_the_callback(
    case_success(&Harness::rejecting(Rejection::Increments), &SuccessEvent {
        call_type: None,
        total_tokens: Some(100),
        request: request(),
        deployment: Some(&usage_limited()),
    }, NOW_MILLIS),
    Evidence::default()
)]
#[case::response_usage_failure_clears_the_marker(
    case_response(&Harness::rejecting(Rejection::Pipeline), request(), Some(&usage_limited()), 25, None),
    Evidence::default()
)]
#[case::cancelled_request_keeps_the_usage_once(
    case_response_then_success(60, None, false),
    Evidence { tpm: Some(60), rpm: Some(1), ..Evidence::default() }
)]
#[case::a_cleared_marker_counts_the_success_in_full(
    case_response_then_success(60, Some(100), true),
    Evidence { key: Some(tpm_key("22-13")), successes: Some(1), tpm: Some(160), rpm: Some(2), ..Evidence::default() }
)]
#[case::failure_counter_failure_is_reported(
    case_failure(&Harness::rejecting(Rejection::Increments), &failure(FailureClass::RateLimit, ErrorStatus::Code(429)), (None, None, None), Some("dep-1"), None),
    Evidence { error: Some("counter:dep-1:fails".to_owned()), ..Evidence::default() }
)]
#[case::failure_cooldown_failure_is_reported(
    case_failure(&Harness::rejecting(Rejection::Cooldowns), &failure(FailureClass::RateLimit, ErrorStatus::Code(429)), (None, None, None), Some("dep-1"), None),
    Evidence { failures: Some(1), error: Some("cooldown:dep-1".to_owned()), ..Evidence::default() }
)]
#[case::failure_without_a_deployment_counts_nothing(
    case_failure(&Harness::default(), &failure(FailureClass::RateLimit, ErrorStatus::Code(429)), (None, None, None), None, None),
    Evidence { cooldown: Some(false), ..Evidence::default() }
)]
#[case::advisor_orchestration_is_not_the_deployment(case_failure(&Harness::default(), &FailureFacts {
    advisor_orchestration: true,
    ..failure(FailureClass::RateLimit, ErrorStatus::Code(429))
}, (None, None, None), Some("dep-1"), None), Evidence { cooldown: Some(false), ..Evidence::default() })]
#[case::background_cost_poll_404_is_not_the_deployment(case_failure(&Harness::default(), &FailureFacts {
    background_response_cost_poll: true,
    ..failure(FailureClass::NotFound, ErrorStatus::Code(404))
}, (None, None, None), Some("dep-1"), None), Evidence { cooldown: Some(false), ..Evidence::default() })]
#[case::a_background_cost_poll_500_is_the_deployment(case_failure(&Harness::default(), &FailureFacts {
    background_response_cost_poll: true,
    ..failure(FailureClass::InternalServer, ErrorStatus::Code(500))
}, (None, None, None), Some("dep-1"), None), Evidence { failures: Some(1), cooldown: Some(false), time_to_cooldown: Some(Seconds(5.0)), minute: Some((0, 1)), ..Evidence::default() })]
#[case::a_plain_404_is_the_deployment(case_failure(&Harness::default(), &failure(FailureClass::NotFound, ErrorStatus::Code(404)), (None, None, None), Some("dep-1"), None), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(5.0)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::a_caller_timeout_at_the_limit_is_not_the_deployment(case_failure(&Harness::default(), &FailureFacts {
    caller_timeout: Some(TimeoutFacts { client_side_timeout: true, timeout_seconds: Some(Seconds(5.0)), elapsed_seconds: Some(Seconds(5.0)) }),
    ..failure(FailureClass::Timeout, ErrorStatus::Code(408))
}, (None, None, None), Some("dep-1"), None), Evidence { cooldown: Some(false), ..Evidence::default() })]
#[case::a_caller_timeout_under_the_limit_is_the_deployment(case_failure(&Harness::default(), &FailureFacts {
    caller_timeout: Some(TimeoutFacts { client_side_timeout: true, timeout_seconds: Some(Seconds(5.0)), elapsed_seconds: Some(Seconds(4.0)) }),
    ..failure(FailureClass::Timeout, ErrorStatus::Code(408))
}, (None, None, None), Some("dep-1"), None), Evidence { failures: Some(1), cooldown: Some(false), time_to_cooldown: Some(Seconds(5.0)), minute: Some((0, 1)), ..Evidence::default() })]
#[case::a_provider_timeout_is_the_deployment(case_failure(&Harness::default(), &FailureFacts {
    caller_timeout: Some(TimeoutFacts { client_side_timeout: false, timeout_seconds: Some(Seconds(5.0)), elapsed_seconds: Some(Seconds(9.0)) }),
    ..failure(FailureClass::Timeout, ErrorStatus::Code(408))
}, (None, None, None), Some("dep-1"), None), Evidence { failures: Some(1), cooldown: Some(false), time_to_cooldown: Some(Seconds(5.0)), minute: Some((0, 1)), ..Evidence::default() })]
#[case::model_info_cooldown_time_wins(case_failure(&Harness::default(), &failure(FailureClass::RateLimit, ErrorStatus::Code(429)), (Some(Seconds(1.5)), Some(Seconds(2.5)), Some(RetryAfter::Seconds(7))), Some("dep-1"), None), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(1.5)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::litellm_params_cooldown_time_applies(case_failure(&Harness::default(), &failure(FailureClass::RateLimit, ErrorStatus::Code(429)), (None, Some(Seconds(2.5)), Some(RetryAfter::Seconds(7))), Some("dep-1"), None), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(2.5)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::a_negative_model_info_value_falls_to_the_header(case_failure(&Harness::default(), &failure(FailureClass::RateLimit, ErrorStatus::Code(429)), (Some(Seconds(-1.0)), Some(Seconds(2.5)), Some(RetryAfter::Seconds(7))), Some("dep-1"), None), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(7.0)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::a_negative_litellm_params_value_falls_to_the_header(case_failure(&Harness::default(), &failure(FailureClass::RateLimit, ErrorStatus::Code(429)), (None, Some(Seconds(-1.0)), Some(RetryAfter::Seconds(4))), Some("dep-1"), None), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(4.0)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::a_negative_header_falls_to_the_router_default(case_failure(&Harness::default(), &failure(FailureClass::RateLimit, ErrorStatus::Code(429)), (None, None, Some(RetryAfter::Seconds(-1))), Some("dep-1"), None), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(5.0)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::a_header_date_counts_from_the_probe_clock(case_failure(&Harness::default(), &failure(FailureClass::RateLimit, ErrorStatus::Code(429)), (None, None, Some(RetryAfter::At { unix_millis: 1_700_000_003_500 })), Some("dep-1"), None), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(3.0)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::a_zero_cooldown_time_is_kept(case_failure(&Harness::default(), &failure(FailureClass::RateLimit, ErrorStatus::Code(429)), (Some(Seconds(0.0)), None, Some(RetryAfter::Seconds(7))), Some("dep-1"), None), Evidence {
    failures: Some(1),
    cooldown: Some(false),
    time_to_cooldown: Some(Seconds(0.0)),
    minute: Some((0, 1)),
    ..Evidence::default()
})]
#[case::last_minute_of_the_hour(case_success(&Harness::default(), &SuccessEvent {
    call_type: None,
    total_tokens: Some(1),
    request: request(),
    deployment: Some(&usage_limited()),
}, 1_700_006_399_999), Evidence {
    key: Some(tpm_key("23-59")),
    successes: Some(1),
    ..Evidence::default()
})]
#[case::first_minute_of_the_hour(case_success(&Harness::default(), &SuccessEvent {
    call_type: None,
    total_tokens: Some(1),
    request: request(),
    deployment: Some(&usage_limited()),
}, 1_700_006_400_000), Evidence {
    key: Some(tpm_key("00-00")),
    successes: Some(1),
    ..Evidence::default()
})]
#[case::single_digit_hour_is_padded(case_success(&Harness::default(), &SuccessEvent {
    call_type: None,
    total_tokens: Some(1),
    request: request(),
    deployment: Some(&usage_limited()),
}, 1_699_952_700_000), Evidence {
    key: Some(tpm_key("09-05")),
    successes: Some(1),
    ..Evidence::default()
})]
#[case::update_usage_saturates(case_update_usage(&Harness::default(), Some(i64::MAX), 1), Evidence {
    request_count: Some(i64::MAX),
    writes: vec![Write { value: i64::MAX, ttl: None }, Write { value: i64::MAX, ttl: None }],
    ..Evidence::default()
})]
#[case::async_failure_skips_a_batch_retrieve(
    case_async_failure(&Harness::default(), &AsyncFailureEvent {
        call_type: Some("retrieve_batch"),
        model_group: Some("probe-group"),
        deployment_name: "openai/gpt-4o",
        deployment_id: Some("dep-1"),
    }, 1),
    Evidence::default()
)]
#[case::async_failure_skips_an_async_batch_retrieve(
    case_async_failure(&Harness::default(), &AsyncFailureEvent {
        call_type: Some("aretrieve_batch"),
        model_group: Some("probe-group"),
        deployment_name: "openai/gpt-4o",
        deployment_id: Some("dep-1"),
    }, 1),
    Evidence::default()
)]
#[case::async_failure_without_a_model_group_counts_nothing(
    case_async_failure(&Harness::default(), &AsyncFailureEvent {
        call_type: None,
        model_group: None,
        deployment_name: "openai/gpt-4o",
        deployment_id: Some("dep-1"),
    }, 1),
    Evidence::default()
)]
#[case::async_failure_without_a_deployment_id_counts_nothing(
    case_async_failure(&Harness::default(), &AsyncFailureEvent {
        call_type: None,
        model_group: Some("probe-group"),
        deployment_name: "openai/gpt-4o",
        deployment_id: None,
    }, 1),
    Evidence::default()
)]
#[case::response_without_a_model_group_marks_nothing(
    case_response(&Harness::default(), RequestFacts { model_group: None, ..request() }, Some(&usage_limited()), 25, None),
    Evidence::default()
)]
#[case::response_without_a_deployment_id_marks_nothing(
    case_response(&Harness::default(), RequestFacts { deployment_id: None, ..request() }, Some(&usage_limited()), 25, None),
    Evidence::default()
)]
#[case::success_usage_failure_keeps_the_success_count(
    case_success(&Harness::rejecting(Rejection::Pipeline), &SuccessEvent {
        call_type: None,
        total_tokens: Some(100),
        request: request(),
        deployment: Some(&usage_limited()),
    }, NOW_MILLIS),
    Evidence { successes: Some(1), ..Evidence::default() }
)]
fn router_observations_boundaries(#[case] actual: Evidence, #[case] expected: Evidence) {
    assert_eq!(actual, expected);
}
