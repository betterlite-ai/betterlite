use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::routing_state_store::connection::Connection;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use tokio::runtime::{Builder, Runtime};

use crate::affinity_claims::identity::{AffinityFlag, AffinityKey};
use crate::affinity_claims::local::LocalPins;
use crate::affinity_claims::pin::AffinityPin;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::redis_failure_policy::Settings as BreakerSettings;
use crate::routing_state_store::Settings as SharedSettings;
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_state_store::{RedisSettings, RoutingStateStore};
use crate::routing_value_store::{Clock, StoreSettings};

#[derive(Clone, Default)]
pub struct TestClock(Arc<Mutex<f64>>);

impl TestClock {
    pub fn at(&self, seconds: f64) {
        let mut now = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *now = seconds;
    }

    pub fn clock(&self) -> Clock {
        let source = Arc::clone(&self.0);
        Arc::new(move || *source.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

pub fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

pub const fn local_settings() -> StoreSettings {
    StoreSettings {
        max_size_in_memory: None,
        default_ttl_seconds: None,
        max_size_per_item: Some(1024),
        loaded_max_size_per_item_kib: 1024,
    }
}

pub fn local_pins(clock: &TestClock) -> LocalPins {
    LocalPins::new(local_settings(), clock.clock()).unwrap()
}

pub fn pin(field: &str, value: &str) -> AffinityPin {
    let mut fields = IndexMap::new();
    fields.insert(field.to_owned(), value.to_owned());
    AffinityPin(fields)
}

pub fn pin_fields(fields: &[(&str, &str)]) -> AffinityPin {
    let mut map = IndexMap::with_capacity(fields.len());
    for (field, value) in fields {
        map.insert((*field).to_owned(), (*value).to_owned());
    }
    AffinityPin(map)
}

pub fn key(raw: &str) -> AffinityKey {
    AffinityKey::try_from(raw).unwrap()
}

pub fn metadata(fields: &[(&str, Json)]) -> JsonObject {
    let mut metadata = JsonObject::new();
    for (field, value) in fields {
        metadata.insert((*field).to_owned(), value.clone());
    }
    metadata
}

pub fn deployment(model_name: &str, params: &[(&str, &str)], id: &str) -> DeploymentSpec {
    let mut litellm_params = JsonObject::new();
    for (field, value) in params {
        litellm_params.insert((*field).to_owned(), Json::Text((*value).to_owned()));
    }
    let mut model_info = JsonObject::new();
    model_info.insert("id".to_owned(), Json::Text(id.to_owned()));
    DeploymentSpec::new(
        model_name.to_owned(),
        litellm_params,
        model_info,
        JsonObject::new(),
        || "generated".to_owned(),
    )
    .unwrap()
}

pub fn flags(names: &[&str]) -> BTreeSet<AffinityFlag> {
    names
        .iter()
        .filter_map(|name| AffinityFlag::from_name(name))
        .collect()
}

pub fn shared_store(server: &FakeRedis) -> RoutingStateStore {
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
    RoutingStateStore::new(&settings, Arc::new(|| Duration::ZERO)).unwrap()
}
