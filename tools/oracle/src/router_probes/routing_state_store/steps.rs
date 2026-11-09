use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use betterlite_router::redis_failure_policy::Settings as BreakerSettings;
use betterlite_router::routing_state_store::error::StateStoreError;
use betterlite_router::routing_state_store::{
    BreakerClock, RedisSettings, RoutingStateStore, Settings,
};
use betterlite_router::routing_value_store::{RouteIncrement, RouteKey, RouteValue, StoreRead};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::SecretString;

use super::fake_redis::FakeRedis;
use super::{Input, Output, Step, StepOutput, StepValue};
use crate::router_probes::error::OperationError;

const WORKERS: usize = 2;

pub(crate) async fn scenario(input: &Input) -> Result<Output, OperationError> {
    let server = FakeRedis::start()
        .await
        .map_err(|error| invalid(&error.to_string()))?;
    let clock = Arc::new(AtomicU64::new(0));
    let settings = settings(input, &server);
    let breaker_clock = breaker_clock(&clock);
    let mut stores = Vec::with_capacity(WORKERS);
    for _worker in 0..WORKERS {
        stores.push(
            RoutingStateStore::new(&settings, Arc::clone(&breaker_clock))
                .map_err(|error| invalid(&error.to_string()))?,
        );
    }
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        clock.store(step.now_millis, Ordering::SeqCst);
        steps.push(apply(step, &stores, &server, input).await?);
    }
    Ok(Output { steps })
}

fn settings(input: &Input, server: &FakeRedis) -> Settings {
    Settings {
        redis: RedisSettings {
            url: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(i64::from(server.addr.port())),
            password: input
                .settings
                .password
                .as_ref()
                .map(|password| SecretString::from(password.clone())),
            namespace: input.settings.namespace.clone(),
            socket_timeout: Some(2.0_f64),
            pool_timeout_seconds: 5,
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

fn breaker_clock(clock: &Arc<AtomicU64>) -> BreakerClock {
    let source = Arc::clone(clock);
    let read = move || Duration::from_millis(source.load(Ordering::SeqCst));
    Arc::new(read)
}

async fn apply(
    step: &Step,
    stores: &[RoutingStateStore],
    server: &FakeRedis,
    input: &Input,
) -> Result<StepOutput, OperationError> {
    match step.kind.as_str() {
        "increment" => increment(step, stores).await,
        "claim" => claim(step, stores).await,
        "get" => get(step, stores).await,
        "ttl" => ttl(step, stores).await,
        "fail" => {
            server.down.store(true, Ordering::SeqCst);
            Ok(StepOutput {
                kind: step.kind.clone(),
                value: None,
                error: None,
            })
        }
        "recover" => {
            server.down.store(false, Ordering::SeqCst);
            Ok(StepOutput {
                kind: step.kind.clone(),
                value: None,
                error: None,
            })
        }
        "leak" => leak(step, stores, input).await,
        other => Err(invalid(&format!("unknown step {other}"))),
    }
}

async fn increment(
    step: &Step,
    stores: &[RoutingStateStore],
) -> Result<StepOutput, OperationError> {
    let store = store_at(stores, step)?;
    let raw = step
        .key
        .as_deref()
        .ok_or_else(|| invalid("the increment needs a key"))?;
    let key = RouteKey::parse(raw).map_err(|error| invalid(&error.to_string()))?;
    let by = step
        .by
        .as_ref()
        .ok_or_else(|| invalid("the increment needs an amount"))?;
    let output = match store.increment(&key, by, None).await {
        Ok(observed) => StepOutput {
            kind: step.kind.clone(),
            value: Some(StepValue::Route(observed.value)),
            error: None,
        },
        Err(error) => StepOutput {
            kind: step.kind.clone(),
            value: None,
            error: Some(label(&error)),
        },
    };
    Ok(output)
}

async fn claim(step: &Step, stores: &[RoutingStateStore]) -> Result<StepOutput, OperationError> {
    let store = store_at(stores, step)?;
    let raw = step
        .key
        .as_deref()
        .ok_or_else(|| invalid("the claim needs a key"))?;
    let key = RouteKey::parse(raw).map_err(|error| invalid(&error.to_string()))?;
    let value = step
        .value
        .as_ref()
        .ok_or_else(|| invalid("the claim needs a value"))?;
    let ttl = step
        .ttl_seconds
        .and_then(|seconds| Decimal::from(seconds).to_f64());
    let output = match store
        .compare_and_set(&key, &StoreRead::Missing, value, ttl)
        .await
    {
        Ok(observed) => StepOutput {
            kind: step.kind.clone(),
            value: Some(StepValue::Claim {
                claimed: observed.value.swapped,
                current: route_value(observed.value.current),
            }),
            error: None,
        },
        Err(error) => StepOutput {
            kind: step.kind.clone(),
            value: None,
            error: Some(label(&error)),
        },
    };
    Ok(output)
}

async fn get(step: &Step, stores: &[RoutingStateStore]) -> Result<StepOutput, OperationError> {
    let store = store_at(stores, step)?;
    let raw = step
        .key
        .as_deref()
        .ok_or_else(|| invalid("the get needs a key"))?;
    let key = RouteKey::parse(raw).map_err(|error| invalid(&error.to_string()))?;
    let output = match store.get(&key).await {
        Ok(observed) => StepOutput {
            kind: step.kind.clone(),
            value: route_value(observed.value).map(StepValue::Route),
            error: None,
        },
        Err(error) => StepOutput {
            kind: step.kind.clone(),
            value: None,
            error: Some(label(&error)),
        },
    };
    Ok(output)
}

async fn ttl(step: &Step, stores: &[RoutingStateStore]) -> Result<StepOutput, OperationError> {
    let store = store_at(stores, step)?;
    let raw = step
        .key
        .as_deref()
        .ok_or_else(|| invalid("the ttl needs a key"))?;
    let key = RouteKey::parse(raw).map_err(|error| invalid(&error.to_string()))?;
    let output = match store.get_ttl(&key).await {
        Ok(observed) => StepOutput {
            kind: step.kind.clone(),
            value: observed.value.map(StepValue::Count),
            error: None,
        },
        Err(error) => StepOutput {
            kind: step.kind.clone(),
            value: None,
            error: Some(label(&error)),
        },
    };
    Ok(output)
}

async fn leak(
    step: &Step,
    stores: &[RoutingStateStore],
    input: &Input,
) -> Result<StepOutput, OperationError> {
    let store = store_at(stores, step)?;
    let raw = step
        .key
        .as_deref()
        .ok_or_else(|| invalid("the leak check needs a key"))?;
    let key = RouteKey::parse(raw).map_err(|error| invalid(&error.to_string()))?;
    let failure = store
        .increment(&key, &RouteIncrement::Counter(1), None)
        .await;
    let leaked = match failure {
        Ok(_observed) => false,
        Err(error) => format!("{error} {error:?}")
            .contains(input.settings.password.as_deref().unwrap_or_default()),
    };
    Ok(StepOutput {
        kind: step.kind.clone(),
        value: Some(StepValue::Flag(leaked)),
        error: None,
    })
}

fn store_at<'stores>(
    stores: &'stores [RoutingStateStore],
    step: &Step,
) -> Result<&'stores RoutingStateStore, OperationError> {
    stores
        .get(step.worker)
        .ok_or_else(|| invalid(&format!("unknown worker {}", step.worker)))
}

fn route_value(read: StoreRead) -> Option<RouteValue> {
    match read {
        StoreRead::Missing => None,
        StoreRead::Value(value) => Some(value),
    }
}

fn label(error: &StateStoreError) -> String {
    match error {
        StateStoreError::CircuitOpen { .. } => "circuit_open",
        StateStoreError::InvalidTtl { .. } => "invalid_ttl",
        StateStoreError::Settings(_)
        | StateStoreError::Factory(_)
        | StateStoreError::Reuse(_)
        | StateStoreError::Redis { .. }
        | StateStoreError::Counters { .. }
        | StateStoreError::Store(_)
        | StateStoreError::Reply { .. }
        | StateStoreError::Encoding { .. }
        | StateStoreError::Conflict { .. }
        | StateStoreError::UnsupportedTarget
        | StateStoreError::NoConnection
        | StateStoreError::NotConfigured => "unavailable",
    }
    .to_owned()
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
