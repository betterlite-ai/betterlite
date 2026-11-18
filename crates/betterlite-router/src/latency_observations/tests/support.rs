use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rust_decimal::Decimal;

use crate::cooldown_decisions::failure::FailureClass;
use crate::latency_observations::LatencyObservations;
use crate::latency_observations::event::{
    Channel, FailureEvent, RequestChannels, SuccessEvent, TokenUsage,
};
use crate::latency_observations::samples::{LatencySamples, MinuteUsage, ModelGroupSamples};
use crate::latency_observations::settings::Settings;
use crate::routing_value_store::Clock;

pub(crate) const GROUP: &str = "gpt";
pub(crate) const OTHER_GROUP: &str = "claude";
pub(crate) const DEPLOYMENT: &str = "dep-a";

pub(crate) struct TestClock {
    millis: AtomicU64,
}

impl TestClock {
    pub(crate) fn at(millis: u64) -> Arc<Self> {
        Arc::new(Self {
            millis: AtomicU64::new(millis),
        })
    }

    pub(crate) fn set(&self, millis: u64) {
        self.millis.store(millis, Ordering::SeqCst);
    }

    pub(crate) fn clock(self: &Arc<Self>) -> Clock {
        let source = Arc::clone(self);
        Arc::new(move || Duration::from_millis(source.millis.load(Ordering::SeqCst)).as_secs_f64())
    }
}

pub(crate) fn service(settings: Settings, clock: &Arc<TestClock>) -> LatencyObservations {
    LatencyObservations::new(settings, clock.clock()).expect("the test cache starts")
}

pub(crate) fn seconds(millis: u64) -> Decimal {
    Decimal::new(i64::try_from(millis).expect("the test time fits i64"), 3)
}

#[derive(Clone, Debug)]
pub(crate) struct Success<'event> {
    pub(crate) call_type: Option<&'event str>,
    pub(crate) channels: RequestChannels<'event>,
    pub(crate) deployment_id: Option<&'event str>,
    pub(crate) stream: bool,
    pub(crate) start_millis: u64,
    pub(crate) elapsed_millis: u64,
    pub(crate) completion_start_millis: Option<u64>,
    pub(crate) usage: Option<TokenUsage>,
}

pub(crate) const DEFAULT_SUCCESS: Success<'static> = Success {
    call_type: None,
    channels: RequestChannels {
        litellm_metadata: Channel::Absent,
        metadata: Channel::Present(Some(GROUP)),
    },
    deployment_id: Some(DEPLOYMENT),
    stream: false,
    start_millis: 0,
    elapsed_millis: 1_000,
    completion_start_millis: None,
    usage: None,
};

impl Default for Success<'_> {
    fn default() -> Self {
        DEFAULT_SUCCESS
    }
}

impl Success<'_> {
    pub(crate) fn event(&self) -> SuccessEvent<'_> {
        SuccessEvent {
            call_type: self.call_type,
            channels: self.channels,
            deployment_id: self.deployment_id,
            stream: self.stream,
            start_time: seconds(self.start_millis),
            end_time: seconds(
                self.start_millis
                    .checked_add(self.elapsed_millis)
                    .expect("the test time fits"),
            ),
            completion_start_time: self.completion_start_millis.map(|millis| {
                seconds(
                    self.start_millis
                        .checked_add(millis)
                        .expect("the test time fits"),
                )
            }),
            usage: self.usage,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Failure<'event> {
    pub(crate) call_type: Option<&'event str>,
    pub(crate) channels: RequestChannels<'event>,
    pub(crate) deployment_id: Option<&'event str>,
    pub(crate) failure: FailureClass,
}

pub(crate) const DEFAULT_FAILURE: Failure<'static> = Failure {
    call_type: None,
    channels: RequestChannels {
        litellm_metadata: Channel::Absent,
        metadata: Channel::Present(Some(GROUP)),
    },
    deployment_id: Some(DEPLOYMENT),
    failure: FailureClass::Timeout,
};

impl Default for Failure<'_> {
    fn default() -> Self {
        DEFAULT_FAILURE
    }
}

impl Failure<'_> {
    pub(crate) fn event(&self) -> FailureEvent<'_> {
        FailureEvent {
            call_type: self.call_type,
            channels: self.channels,
            deployment_id: self.deployment_id,
            failure: self.failure,
        }
    }
}

pub(crate) fn record_success(service: &LatencyObservations, plan: &Success<'_>) {
    service
        .record_success(&plan.event())
        .expect("the test success records");
}

pub(crate) fn record_elapsed(service: &LatencyObservations, values: &[u64]) {
    for value in values {
        record_success(service, &success_plan(*value, None));
    }
}

pub(crate) fn success_plan(elapsed_millis: u64, usage: Option<TokenUsage>) -> Success<'static> {
    Success {
        elapsed_millis,
        usage,
        ..DEFAULT_SUCCESS
    }
}

pub(crate) fn record_failure(service: &LatencyObservations, plan: &Failure<'_>) {
    service
        .record_failure(&plan.event())
        .expect("the test failure records");
}

pub(crate) fn samples(service: &LatencyObservations, group: &str) -> ModelGroupSamples {
    service.samples(group).expect("the test samples read")
}

pub(crate) fn deployment(samples: &ModelGroupSamples, id: &str) -> LatencySamples {
    samples.deployments.get(id).cloned().unwrap_or_default()
}

pub(crate) fn latency(service: &LatencyObservations) -> Vec<f64> {
    deployment(&samples(service, GROUP), DEPLOYMENT).latency
}

pub(crate) fn ttft(service: &LatencyObservations) -> Vec<f64> {
    deployment(&samples(service, GROUP), DEPLOYMENT).time_to_first_token_seconds
}

pub(crate) fn minutes(service: &LatencyObservations) -> Vec<(String, MinuteUsage)> {
    deployment(&samples(service, GROUP), DEPLOYMENT)
        .minutes
        .into_iter()
        .collect()
}

pub(crate) fn usage(completion_tokens: i64, total_tokens: i64) -> Option<TokenUsage> {
    Some(TokenUsage {
        completion_tokens,
        total_tokens,
    })
}
