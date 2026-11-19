use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::routing_state_store::connection::Connection;
use betterlite_router::affinity_claims::AffinityClaims;
use betterlite_router::affinity_claims::error::AffinityError;
use betterlite_router::affinity_claims::identity::{
    AffinityKey, affinity_cache_key, session_affinity_cache_key,
};
use betterlite_router::affinity_claims::local::LocalPins;
use betterlite_router::affinity_claims::pin::StoredPin;
use betterlite_router::affinity_claims::shared::SharedFallback;
use betterlite_router::redis_failure_policy::Settings as BreakerSettings;
use betterlite_router::routing_state_store::Settings as SharedSettings;
use betterlite_router::routing_state_store::test_support::FakeRedis;
use betterlite_router::routing_state_store::{RedisSettings, RoutingStateStore};
use betterlite_router::routing_value_store::{
    PinValue, RouteKey, RouteValue, StoreError, StoreRead, StoreSettings,
};

use super::{
    Input, KeyKind, KeyRequest, Observation, Output, PinField, SeedTarget, Step, build_pin,
    claim_fields, invalid, parse_step,
};
use crate::router_probes::error::OperationError;

struct Scenario {
    service: AffinityClaims,
    server: Option<FakeRedis>,
}

pub(super) async fn execute(input: &Input) -> Result<Output, OperationError> {
    let server = if input.shared {
        Some(
            FakeRedis::try_start()
                .await
                .map_err(|error| invalid(&error.to_string()))?,
        )
    } else {
        None
    };
    let shared = server.as_ref().map(shared_store).transpose()?;
    let local = LocalPins::new(local_settings(), Arc::new(|| 0.0_f64))
        .map_err(|error| invalid(&error.to_string()))?;
    let scenario = Scenario {
        service: AffinityClaims { local, shared },
        server,
    };
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        steps.push(apply_step(step, &scenario).await?);
    }
    Ok(Output { steps })
}

async fn apply_step(step: &Step, scenario: &Scenario) -> Result<Observation, OperationError> {
    match step {
        Step::Key { request } => key_step(request),
        Step::Claim {
            key,
            pin,
            ttl_seconds,
            down,
            eligible,
        } => {
            if let Some(server) = &scenario.server {
                server.set_down(*down);
            }
            claim(scenario, key, pin, *ttl_seconds, eligible.as_deref()).await
        }
        Step::Seed {
            target,
            key,
            raw,
            ttl_seconds,
        } => seed(scenario, *target, key, raw, *ttl_seconds).await,
        Step::Parse { pad_len } => parse_step(*pad_len),
    }
}

fn key_step(request: &KeyRequest) -> Result<Observation, OperationError> {
    let built = match request.kind {
        KeyKind::Affinity => {
            let Some(user_key) = &request.user_key else {
                return Err(invalid("an affinity key needs a user key"));
            };
            affinity_cache_key(&request.model_group, user_key)
        }
        KeyKind::Session => {
            let Some(session_id) = &request.session_id else {
                return Err(invalid("a session key needs a session id"));
            };
            session_affinity_cache_key(
                &request.model_group,
                session_id,
                request.user_key.as_deref(),
            )
        }
    };
    match built {
        Ok(built_key) => Ok(Observation::Key {
            key: Some(built_key.as_ref().to_owned()),
        }),
        Err(AffinityError::Store(StoreError::KeyTooLong { .. })) => {
            Ok(Observation::Key { key: None })
        }
        Err(error) => Err(invalid(&error.to_string())),
    }
}

async fn claim(
    scenario: &Scenario,
    key: &str,
    fields: &[PinField],
    ttl_seconds: u32,
    eligible: Option<&[Vec<PinField>]>,
) -> Result<Observation, OperationError> {
    let mut candidates = None;
    if let Some(sets) = eligible {
        let mut pins = Vec::with_capacity(sets.len());
        for entry in sets {
            pins.push(build_pin(entry));
        }
        candidates = Some(pins);
    }
    let outcome = Box::pin(scenario.service.claim(
        &AffinityKey::try_from(key).map_err(|error| invalid(&error.to_string()))?,
        &build_pin(fields),
        ttl_seconds,
        candidates.as_deref(),
    ))
    .await
    .map_err(|error| invalid(&error.to_string()))?;
    let (pin_kind, model_id, text) = claim_fields(&outcome.pin);
    Ok(Observation::Claim {
        pin_kind,
        model_id,
        text,
        fallback: outcome.fallback.map(fallback_name),
    })
}

async fn seed(
    scenario: &Scenario,
    target: SeedTarget,
    key: &str,
    raw: &str,
    ttl_seconds: u32,
) -> Result<Observation, OperationError> {
    let parsed = AffinityKey::try_from(key).map_err(|error| invalid(&error.to_string()))?;
    match target {
        SeedTarget::Local => {
            let stored =
                StoredPin::parse(raw.as_bytes()).map_err(|error| invalid(&error.to_string()))?;
            scenario
                .service
                .local
                .set(&parsed, &stored, ttl_seconds)
                .map_err(|error| invalid(&error.to_string()))?;
        }
        SeedTarget::Shared => {
            let Some(store) = scenario.service.shared.as_ref() else {
                return Err(invalid("the shared store is not configured"));
            };
            let route = RouteKey::parse(key).map_err(|error| invalid(&error.to_string()))?;
            let value = RouteValue::Pin(PinValue {
                model_id: raw.to_owned(),
            });
            Box::pin(store.compare_and_set(
                &route,
                &StoreRead::Missing,
                &value,
                Some(f64::from(ttl_seconds)),
            ))
            .await
            .map_err(|error| invalid(&error.to_string()))?;
        }
    }
    Ok(Observation::Seed)
}

fn fallback_name(fallback: SharedFallback) -> String {
    match fallback {
        SharedFallback::Unavailable => "unavailable".to_owned(),
        SharedFallback::Conflict => "conflict".to_owned(),
        SharedFallback::ValueUnusable => "value_unusable".to_owned(),
    }
}

const fn local_settings() -> StoreSettings {
    StoreSettings {
        max_size_in_memory: None,
        default_ttl_seconds: None,
        max_size_per_item: Some(1024),
        loaded_max_size_per_item_kib: 1024,
    }
}

fn shared_store(server: &FakeRedis) -> Result<RoutingStateStore, OperationError> {
    let settings = SharedSettings {
        redis: RedisSettings {
            url: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(i64::from(server.port())),
            password: None,
            namespace: None,
            connection: Connection::default(),
            socket_timeout: 0.25,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: BreakerSettings {
            enabled: true,
            failure_threshold: 100,
            recovery_timeout: Duration::from_secs(60),
            timeout_min_duration: Duration::from_secs(5),
            timeout_log_interval: Duration::from_secs(1),
        },
    };
    RoutingStateStore::new(&settings, Arc::new(|| Duration::ZERO))
        .map_err(|error| invalid(&error.to_string()))
}
