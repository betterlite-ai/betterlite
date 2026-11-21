use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::routing_state_store::connection::Connection;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::cooldown_decisions::failure::{ErrorStatus, FailureClass, FailureFacts, Seconds};
use crate::cooldown_decisions::policy::{CooldownPolicy, FailureThresholds};
use crate::cooldown_policy::counter::AllowedFailsStore;
use crate::cooldown_policy::event::{CooldownEvent, CooldownObservationError, CooldownObserver};
use crate::cooldown_policy::service::{
    CooldownAttempt, CooldownCall, CooldownPolicyService, FallbackFailure,
};
use crate::cooldown_state::cooldown::CooldownCache;
use crate::cooldown_state::record::CooldownRecord;
use crate::cooldown_state::settings::Settings as CacheSettings;
use crate::cooldown_state::shared::SharedCooldowns;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::alias::AliasMap;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::redis_failure_policy::Settings as BreakerSettings;
use crate::routing_groups::group::RoutingGroup;
use crate::routing_groups::index::RoutingGroupIndex;
use crate::routing_read_batch::store::{BatchRead, BatchReadFuture};
use crate::routing_state::Settings as CounterSettings;
use crate::routing_state::counter::RouterCounters;
use crate::routing_state_store::Settings as RedisStoreSettings;
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_state_store::{BreakerClock, RedisSettings, RoutingStateStore};
use crate::routing_value_store::{Clock, RouteKey, RoutingValueStore, StoreSettings};

pub(super) type SharedClock = Arc<Mutex<f64>>;

#[derive(Default)]
pub(super) struct FakeCooldowns {
    pub records: Mutex<BTreeMap<RouteKey, CooldownRecord>>,
}

impl SharedCooldowns for FakeCooldowns {
    fn batch_read<'store>(
        &'store self,
        keys: &'store [RouteKey],
    ) -> BatchReadFuture<'store, CooldownRecord> {
        Box::pin(async move {
            let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
            let values = keys.iter().map(|key| records.get(key).cloned()).collect();
            BatchRead {
                values,
                warning: None,
            }
        })
    }

    fn write(&self, key: &RouteKey, record: &CooldownRecord, _ttl_seconds: f64) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), record.clone());
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Recorded {
    pub model_name: String,
    pub model_id: String,
    pub api_base: String,
    pub provider: String,
    pub exception_status: String,
}

#[derive(Default)]
pub(super) struct RecordingObserver {
    pub events: Mutex<Vec<Recorded>>,
}

impl RecordingObserver {
    pub(super) fn recorded(&self) -> Vec<Recorded> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CooldownObserver for RecordingObserver {
    fn deployment_cooled_down(
        &self,
        event: &CooldownEvent<'_>,
    ) -> Result<(), CooldownObservationError> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                model_name: event.model_name.to_owned(),
                model_id: event.model_id.to_owned(),
                api_base: event.api_base.to_owned(),
                provider: event.provider.to_owned(),
                exception_status: event.exception_status.to_owned(),
            });
        Ok(())
    }
}

pub(super) struct FailingObserver;

impl CooldownObserver for FailingObserver {
    fn deployment_cooled_down(
        &self,
        _event: &CooldownEvent<'_>,
    ) -> Result<(), CooldownObservationError> {
        Err(CooldownObservationError::Observer {
            message: "the exporter is down".to_owned(),
        })
    }
}

type ProviderFn = Box<dyn Fn(&str) -> Option<String>>;

pub(super) struct Fixture {
    pub catalog: DeploymentCatalog,
    pub aliases: AliasMap,
    pub groups: RoutingGroupIndex,
    provider: ProviderFn,
}

impl Fixture {
    pub(super) fn new(deployments: Vec<DeploymentSpec>) -> Self {
        let mut catalog = DeploymentCatalog::default();
        for deployment in deployments {
            catalog.push(deployment);
        }
        Self {
            catalog,
            aliases: AliasMap::new(),
            groups: RoutingGroupIndex::default(),
            provider: Box::new(|_: &str| None),
        }
    }

    pub(super) fn with_provider_default(mut self, id: &str) -> Self {
        self.catalog.provider_default_ids.push(id.to_owned());
        self
    }

    pub(super) fn with_group(mut self, name: &str, members: &[&str]) -> Self {
        let group = RoutingGroup::new(
            name.to_owned(),
            members.iter().map(|member| (*member).to_owned()).collect(),
            "simple-shuffle",
            None,
            None,
        )
        .unwrap();
        self.groups = RoutingGroupIndex::build(vec![group], &BTreeSet::new(), &BTreeMap::new())
            .unwrap()
            .index;
        self
    }

    pub(super) fn lookup(&self) -> CatalogLookup<'_> {
        CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &*self.provider,
        }
    }
}

pub(super) struct Harness {
    pub service: CooldownPolicyService<RoutingStateStore>,
    pub fixture: Fixture,
    pub clock: SharedClock,
    pub events: Arc<RecordingObserver>,
    pub shared: Arc<FakeCooldowns>,
}

pub(super) fn harness(fixture: Fixture, policy: CooldownPolicy) -> Harness {
    harness_with(fixture, policy, Vec::new())
}

pub(super) fn harness_with(
    fixture: Fixture,
    policy: CooldownPolicy,
    extras: Vec<Arc<dyn CooldownObserver>>,
) -> Harness {
    harness_built(fixture, policy, extras, None)
}

// a harness whose allowed-fails counters also live in the shared store the fake redis serves
pub(super) fn harness_sharing(
    fixture: Fixture,
    policy: CooldownPolicy,
    server: &FakeRedis,
) -> Harness {
    let breaker_clock: BreakerClock = Arc::new(|| Duration::from_millis(0));
    let remote = Arc::new(RoutingStateStore::new(&shared_settings(server), breaker_clock).unwrap());
    harness_built(fixture, policy, Vec::new(), Some(remote))
}

fn shared_settings(server: &FakeRedis) -> RedisStoreSettings {
    RedisStoreSettings {
        redis: RedisSettings {
            url: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(i64::from(server.port())),
            password: None,
            namespace: Some("b308".to_owned()),
            connection: Connection::default(),
            socket_timeout: 2.0_f64,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: BreakerSettings {
            enabled: true,
            failure_threshold: 3,
            recovery_timeout: Duration::ZERO,
            timeout_min_duration: Duration::from_millis(5_000),
            timeout_log_interval: Duration::from_millis(1_000),
        },
    }
}

fn harness_built(
    fixture: Fixture,
    policy: CooldownPolicy,
    extras: Vec<Arc<dyn CooldownObserver>>,
    remote: Option<Arc<RoutingStateStore>>,
) -> Harness {
    let clock: SharedClock = Arc::new(Mutex::new(1000.0_f64));
    let handle = Arc::clone(&clock);
    let clock_fn: Clock = Arc::new(move || *handle.lock().unwrap_or_else(PoisonError::into_inner));
    let shared = Arc::new(FakeCooldowns::default());
    let shared_cooldowns: Arc<dyn SharedCooldowns> = Arc::clone(&shared);
    let cooldowns = CooldownCache::new(
        &CacheSettings::default(),
        shared_cooldowns,
        Arc::clone(&clock_fn),
    );
    let local = Arc::new(
        RoutingValueStore::new(
            StoreSettings {
                max_size_in_memory: None,
                default_ttl_seconds: None,
                max_size_per_item: None,
                loaded_max_size_per_item_kib: 1024,
            },
            clock_fn,
        )
        .unwrap(),
    );
    let counters = RouterCounters::<RoutingStateStore>::new(
        CounterSettings {
            sync_interval: None,
            default_sync_interval: Duration::from_secs(60),
            should_batch_redis_writes: false,
        },
        Arc::clone(&local),
        None,
    );
    let events = Arc::new(RecordingObserver::default());
    let observer: Arc<dyn CooldownObserver> = Arc::clone(&events);
    let mut observers: Vec<Arc<dyn CooldownObserver>> = vec![observer];
    observers.extend(extras);
    let service = CooldownPolicyService::new(
        policy,
        cooldowns,
        counters,
        AllowedFailsStore::new(local, remote),
        observers,
    );
    Harness {
        service,
        fixture,
        clock,
        events,
        shared,
    }
}

pub(super) fn policy(allowed_fails: u32) -> CooldownPolicy {
    CooldownPolicy {
        allowed_fails,
        default_allowed_fails: 3,
        cooldown_time: Seconds(5.0),
        disable_cooldowns: false,
        allowed_fails_policy: None,
        thresholds: FailureThresholds::default(),
    }
}

// the ratio path runs while the router value is still the built-in default
pub(super) fn ratio_policy() -> CooldownPolicy {
    policy(3_u32)
}

pub(super) fn deployment(model_name: &str, id: &str) -> DeploymentSpec {
    deployment_with(model_name, id, Vec::new(), Vec::new())
}

pub(super) fn deployment_with(
    model_name: &str,
    id: &str,
    params: Vec<(&str, Json)>,
    info: Vec<(&str, Json)>,
) -> DeploymentSpec {
    let mut litellm_params = JsonObject::new();
    litellm_params.insert("model".to_owned(), Json::Text("openai/gpt-4o".to_owned()));
    for (key, value) in params {
        litellm_params.insert(key.to_owned(), value);
    }
    let mut model_info = JsonObject::new();
    model_info.insert("id".to_owned(), Json::Text(id.to_owned()));
    for (key, value) in info {
        model_info.insert(key.to_owned(), value);
    }
    DeploymentSpec::new(
        model_name.to_owned(),
        litellm_params,
        model_info,
        JsonObject::new(),
        generated_id,
    )
    .unwrap()
}

fn generated_id() -> String {
    "generated".to_owned()
}

pub(super) const fn attempt<'case>(
    deployment_id: &'case str,
    failure: &'case FailureFacts,
) -> CooldownAttempt<'case> {
    attempt_with(deployment_id, failure, None, None)
}

pub(super) const fn attempt_with<'case>(
    deployment_id: &'case str,
    failure: &'case FailureFacts,
    header_retry_after: Option<Seconds>,
    requested_model_group: Option<&'case str>,
) -> CooldownAttempt<'case> {
    CooldownAttempt {
        deployment_id,
        failure,
        exception: "probe failure",
        header_retry_after,
        requested_model_group,
    }
}

pub(super) const fn call<'case>(
    deployment_id: &'case str,
    failure: &'case FailureFacts,
) -> CooldownCall<'case> {
    CooldownCall {
        deployment_id,
        failure,
        exception: "probe failure",
        time_to_cooldown: None,
        requested_model_group: None,
    }
}

pub(super) const fn fallback<'case>(
    deployment_id: Option<&'case str>,
    failure: &'case FailureFacts,
) -> FallbackFailure<'case> {
    FallbackFailure {
        deployment_id,
        failure,
        exception: "probe failure",
        original_generic_function: false,
        header_retry_after: None,
    }
}

pub(super) const fn failure(class: FailureClass, status: ErrorStatus) -> FailureFacts {
    FailureFacts {
        class,
        status,
        advisor_orchestration: false,
        background_response_cost_poll: false,
        wrapped_exception: None,
        caller_timeout: None,
    }
}

pub(super) const fn rate_limit(status: i64) -> FailureFacts {
    failure(FailureClass::RateLimit, ErrorStatus::Code(status))
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn number(value: u32) -> Json {
    Json::Number(serde_json::Number::from(value))
}

pub(super) fn float(value: f64) -> Json {
    Json::Number(serde_json::Number::from_f64(value).unwrap())
}

pub(super) fn stored_record(shared: &FakeCooldowns, id: &str) -> Option<CooldownRecord> {
    let key = RouteKey::parse(&format!("deployment:{id}:cooldown")).unwrap();
    let records = shared
        .records
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    records.get(&key).cloned()
}
