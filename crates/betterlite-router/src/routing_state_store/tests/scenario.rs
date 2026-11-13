use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use betterlite_config::sections::routing_state_store::connection::Connection;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::error::{Observed, StateStoreError};
use super::test_support::FakeRedis;
use super::{BreakerClock, RedisSettings, RoutingStateStore, Settings};
use crate::redis_failure_policy::Settings as BreakerSettings;
use crate::routing_value_store::{CompareOutcome, RouteIncrement, RouteKey, RouteValue, StoreRead};

const WORKERS: usize = 2;

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioInput {
    pub settings: ScenarioSettings,
    pub steps: Vec<ScenarioStep>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioSettings {
    #[serde(default)]
    pub url: Option<String>,
    pub namespace: Option<String>,
    pub password: Option<String>,
    pub failure_threshold: u32,
    pub recovery_timeout_millis: u64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Increment,
    Claim,
    Fail,
    Recover,
    Leak,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioStep {
    pub kind: StepKind,
    pub now_millis: u64,
    #[serde(default)]
    pub worker: usize,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub by: Option<RouteIncrement>,
    #[serde(default)]
    pub value: Option<RouteValue>,
    #[serde(default)]
    pub ttl_seconds: Option<i64>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioOutput {
    pub steps: Vec<ScenarioStepOutput>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioStepOutput {
    pub kind: StepKind,
    pub value: Option<ScenarioValue>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ScenarioValue {
    Route(RouteValue),
    Claim {
        claimed: bool,
        current: Option<RouteValue>,
    },
    Flag(bool),
}

pub async fn run_scenario(input: &ScenarioInput) -> Result<ScenarioOutput, String> {
    let server = FakeRedis::try_start()
        .await
        .map_err(|error| error.to_string())?;
    let clock = Arc::new(AtomicU64::new(0));
    let settings = settings(input, &server);
    let source = Arc::clone(&clock);
    let breaker_clock: BreakerClock =
        Arc::new(move || Duration::from_millis(source.load(Ordering::SeqCst)));
    let mut stores = Vec::with_capacity(WORKERS);
    for _worker in 0..WORKERS {
        stores.push(
            RoutingStateStore::new(&settings, Arc::clone(&breaker_clock))
                .map_err(|error| error.to_string())?,
        );
    }
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        clock.store(step.now_millis, Ordering::SeqCst);
        steps.push(apply(step, &stores, &server, input).await?);
    }
    Ok(ScenarioOutput { steps })
}

fn settings(input: &ScenarioInput, server: &FakeRedis) -> Settings {
    let url = input.settings.url.clone().map(SecretString::from);
    let named = url.is_none();
    Settings {
        redis: RedisSettings {
            host: named.then(|| "127.0.0.1".to_owned()),
            port: named.then(|| i64::from(server.port())),
            url,
            password: input
                .settings
                .password
                .as_ref()
                .filter(|_| named)
                .map(|password| SecretString::from(password.clone())),
            namespace: input.settings.namespace.clone(),
            connection: Connection::default(),
            socket_timeout: 2.0_f64,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: BreakerSettings {
            enabled: true,
            failure_threshold: input.settings.failure_threshold,
            recovery_timeout: Duration::from_millis(input.settings.recovery_timeout_millis),
            timeout_min_duration: Duration::from_millis(5_000),
            timeout_log_interval: Duration::from_millis(1_000),
        },
    }
}

async fn apply(
    step: &ScenarioStep,
    stores: &[RoutingStateStore],
    server: &FakeRedis,
    input: &ScenarioInput,
) -> Result<ScenarioStepOutput, String> {
    match step.kind {
        StepKind::Increment => increment(step, stores).await,
        StepKind::Claim => claim(step, stores).await,
        StepKind::Fail => {
            server.set_down(true);
            Ok(output(step, None, None))
        }
        StepKind::Recover => {
            server.set_down(false);
            Ok(output(step, None, None))
        }
        StepKind::Leak => leak(step, stores, input).await,
    }
}

#[rustfmt::skip]
async fn increment(step: &ScenarioStep, stores: &[RoutingStateStore]) -> Result<ScenarioStepOutput, String> {
    let (store, key) = request(step, stores)?;
    let by = step.by.as_ref().ok_or_else(|| "the increment needs an amount".to_owned())?;
    let observed = store.increment(&key, by, None).await;
    Ok(route_output(step, observed))
}

#[rustfmt::skip]
fn route_output(step: &ScenarioStep, result: Result<Observed<RouteValue>, StateStoreError>) -> ScenarioStepOutput {
    match result {
        Ok(observed) => output(step, Some(ScenarioValue::Route(observed.value)), None),
        Err(error) => output(step, None, Some(label(&error))),
    }
}

#[rustfmt::skip]
async fn claim(step: &ScenarioStep, stores: &[RoutingStateStore]) -> Result<ScenarioStepOutput, String> {
    let (store, key) = request(step, stores)?;
    let requested = step.value.as_ref().ok_or_else(|| "the claim needs a value".to_owned())?;
    let ttl = step.ttl_seconds.and_then(|seconds| Decimal::from(seconds).to_f64());
    let outcome = store.compare_and_set(&key, &StoreRead::Missing, requested, ttl).await;
    Ok(claim_output(step, outcome))
}

#[rustfmt::skip]
fn claim_output(step: &ScenarioStep, outcome: Result<Observed<CompareOutcome>, StateStoreError>) -> ScenarioStepOutput {
    match outcome {
        Ok(observed) => output(step, Some(ScenarioValue::Claim { claimed: observed.value.swapped, current: current(observed.value.current) }), None),
        Err(error) => output(step, None, Some(label(&error))),
    }
}

pub(crate) fn current(read: StoreRead) -> Option<RouteValue> {
    match read {
        StoreRead::Missing => None,
        StoreRead::Value(value) => Some(value),
    }
}

#[rustfmt::skip]
async fn leak(step: &ScenarioStep, stores: &[RoutingStateStore], input: &ScenarioInput) -> Result<ScenarioStepOutput, String> {
    let (store, key) = request(step, stores)?;
    let failure = store.increment(&key, &RouteIncrement::Counter(1), None).await;
    Ok(output(step, Some(ScenarioValue::Flag(leaked(failure, input.settings.password.as_deref()))), None))
}

pub(crate) fn leaked(
    failure: Result<Observed<RouteValue>, StateStoreError>,
    configured: Option<&str>,
) -> bool {
    match (failure, configured) {
        (Err(error), Some(password)) => format!("{error} {error:?}").contains(password),
        (Ok(_), _) | (Err(_), None) => false,
    }
}

const fn output(
    step: &ScenarioStep,
    value: Option<ScenarioValue>,
    error: Option<String>,
) -> ScenarioStepOutput {
    ScenarioStepOutput {
        kind: step.kind,
        value,
        error,
    }
}

#[rustfmt::skip]
fn request<'stores>(step: &ScenarioStep, stores: &'stores [RoutingStateStore]) -> Result<(&'stores RoutingStateStore, RouteKey), String> {
    let store = stores.get(step.worker).ok_or_else(|| format!("unknown worker {}", step.worker))?;
    let raw = step.key.as_deref().ok_or_else(|| "the step needs a key".to_owned())?;
    Ok((store, RouteKey::parse(raw).map_err(|error| error.to_string())?))
}

pub(crate) fn label(error: &StateStoreError) -> String {
    if matches!(error, StateStoreError::CircuitOpen { .. }) {
        return "circuit_open".to_owned();
    }
    if matches!(error, StateStoreError::InvalidTtl { .. }) {
        return "invalid_ttl".to_owned();
    }
    "unavailable".to_owned()
}
