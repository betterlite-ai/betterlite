use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::cooldown_decisions::policy::{CooldownPolicy, FailureThresholds};
use betterlite_router::cooldown_policy::counter::AllowedFailsStore;
use betterlite_router::cooldown_policy::event::{
    CooldownEvent, CooldownObservationError, CooldownObserver,
};
use betterlite_router::cooldown_policy::service::CooldownPolicyService;
use betterlite_router::cooldown_state::cooldown::CooldownCache;
use betterlite_router::cooldown_state::record::CooldownRecord;
use betterlite_router::cooldown_state::settings::Settings as CooldownSettings;
use betterlite_router::cooldown_state::shared::SharedCooldowns;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::routing_read_batch::store::{BatchRead, BatchReadFuture};
use betterlite_router::routing_state::Settings as CounterSettings;
use betterlite_router::routing_state::counter::RouterCounters;
use betterlite_router::routing_state_store::RoutingStateStore;
use betterlite_router::routing_value_store::{Clock, RouteKey, RoutingValueStore, StoreSettings};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use super::{DeploymentInput, EventOutput, Payload};

pub(super) fn seconds(millis: u64) -> Result<f64, String> {
    Decimal::from(millis)
        .checked_div(Decimal::from(1_000_i64))
        .and_then(|value| value.to_f64())
        .ok_or_else(|| format!("{millis} milliseconds cannot be represented as seconds"))
}

pub(super) fn spec(input: &DeploymentInput) -> Result<DeploymentSpec, String> {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(input.model.clone()));
    params.insert("api_key".to_owned(), Json::Text("sk-probe".to_owned()));
    if let Some(api_base) = &input.api_base {
        params.insert("api_base".to_owned(), Json::Text(api_base.clone()));
    }
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(input.id.clone()));
    DeploymentSpec::new(
        input.model_name.clone(),
        params,
        info,
        JsonObject::new(),
        || "generated".to_owned(),
    )
    .map_err(|error| error.to_string())
}

// router_cooldown_event_callback's five values, recorded from the one typed observer hook
#[derive(Default)]
pub(super) struct Recorder {
    events: Mutex<Vec<EventOutput>>,
}

impl Recorder {
    pub(super) fn events(&self) -> Vec<EventOutput> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CooldownObserver for Recorder {
    fn deployment_cooled_down(
        &self,
        event: &CooldownEvent<'_>,
    ) -> Result<(), CooldownObservationError> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(EventOutput {
                model_name: event.model_name.to_owned(),
                model_id: event.model_id.to_owned(),
                api_base: event.api_base.to_owned(),
                provider: event.provider.to_owned(),
                exception_status: event.exception_status.to_owned(),
            });
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct FakeCooldowns {
    records: Mutex<BTreeMap<RouteKey, CooldownRecord>>,
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

pub(super) struct ProbeFixture {
    pub(super) service: CooldownPolicyService<RoutingStateStore>,
    pub(super) minute_counters: RouterCounters<RoutingStateStore>,
    pub(super) recorder: Arc<Recorder>,
}

impl ProbeFixture {
    pub(super) fn build(payload: &Payload, clock: &Clock) -> Result<Self, String> {
        let store_settings = StoreSettings {
            max_size_in_memory: None,
            default_ttl_seconds: None,
            max_size_per_item: None,
            loaded_max_size_per_item_kib: 1024,
        };
        let store = Arc::new(
            RoutingValueStore::new(store_settings, Arc::clone(clock))
                .map_err(|error| error.to_string())?,
        );
        let counters = ProbeCounters::build(Arc::clone(&store));
        let shared: Arc<dyn SharedCooldowns> = Arc::new(FakeCooldowns::default());
        let cooldowns = CooldownCache::new(&CooldownSettings::default(), shared, Arc::clone(clock));
        let recorder = Arc::new(Recorder::default());
        let observer: Arc<dyn CooldownObserver> = Arc::<Recorder>::clone(&recorder);
        let service = CooldownPolicyService::new(
            CooldownPolicy {
                allowed_fails: payload.router.allowed_fails,
                default_allowed_fails: payload.router.default_allowed_fails,
                cooldown_time: payload.router.cooldown_time,
                disable_cooldowns: payload.router.disable_cooldowns,
                allowed_fails_policy: None,
                thresholds: FailureThresholds::default(),
            },
            cooldowns,
            counters.service,
            AllowedFailsStore {
                local: store,
                remote: None,
            },
            vec![observer],
        );
        Ok(Self {
            service,
            minute_counters: counters.minute,
            recorder,
        })
    }
}

struct ProbeCounters {
    service: RouterCounters<RoutingStateStore>,
    minute: RouterCounters<RoutingStateStore>,
}

impl ProbeCounters {
    fn build(store: Arc<RoutingValueStore>) -> Self {
        let counter_settings = CounterSettings {
            sync_interval: None,
            default_sync_interval: Duration::from_secs(60),
            should_batch_redis_writes: false,
        };
        Self {
            service: RouterCounters::new(counter_settings.clone(), Arc::clone(&store), None),
            minute: RouterCounters::new(counter_settings, store, None),
        }
    }
}
