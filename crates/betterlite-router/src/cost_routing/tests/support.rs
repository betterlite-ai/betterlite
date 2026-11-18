use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::cost_routing::costs::{Catalog, ModelCosts};
use crate::cost_routing::keys::window_keys;
use crate::cost_routing::selection::Selection;
use crate::cost_routing::service::{CostRouter, Entry};
use crate::cost_routing::window::{CostWindows, WindowCounters, WindowUsage};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::io_token_reservations::clock::IoClock;
use crate::routing_state::Settings;
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::delta::{Delta, DeltaStore};
use crate::routing_state::error::RoutingStateError;
use crate::routing_value_store::{Clock, RouteKey, RouteValue, RoutingValueStore, StoreSettings};

pub(super) const GROUP: &str = "probe-group";
pub(super) const MODEL: &str = "openai/probe";
pub(super) const MINUTE: &str = "2026-11-18-12-00";
pub(super) const NEXT_MINUTE: &str = "2026-11-18-12-01";
pub(super) const MINUTE_MILLIS: u64 = 1_795_003_200_000;

#[derive(Default)]
pub(super) struct ProbeCosts {
    entries: BTreeMap<String, JsonObject>,
}

impl ProbeCosts {
    pub(super) fn new(entries: &[(&str, JsonObject)]) -> Self {
        Self {
            entries: entries
                .iter()
                .map(|(model, entry)| ((*model).to_owned(), entry.clone()))
                .collect(),
        }
    }
}

impl ModelCosts for ProbeCosts {
    fn entry(&self, model: &str) -> Option<Catalog<'_>> {
        self.entries.get(model).map(Catalog::plain)
    }
}

pub(super) fn dec(value: &str) -> Decimal {
    value.parse().unwrap()
}

pub(super) fn number(value: &str) -> Json {
    Json::Number(Number::from_str(value).unwrap())
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn object(entries: &[(&str, Json)]) -> JsonObject {
    let mut map = JsonObject::new();
    for (key, value) in entries {
        map.insert((*key).to_owned(), value.clone());
    }
    map
}

pub(super) fn deployment(id: &str) -> DeploymentSpec {
    spec(id, MODEL, &[], &[], &[])
}

// A deployment whose model_info is its catalog entry, with the prices, limits and top-level keys a case gives litellm_params.
pub(super) fn spec(
    id: &str,
    model: &str,
    params: &[(&str, Json)],
    info: &[(&str, Json)],
    extra: &[(&str, Json)],
) -> DeploymentSpec {
    let mut litellm_params = object(&[("model", text(model))]);
    for (key, value) in params {
        litellm_params.insert((*key).to_owned(), value.clone());
    }
    let mut model_info = object(&[("id", text(id))]);
    for (key, value) in info {
        model_info.insert((*key).to_owned(), value.clone());
    }
    let mut top_level = JsonObject::new();
    for (key, value) in extra {
        top_level.insert((*key).to_owned(), value.clone());
    }
    DeploymentSpec::new(
        GROUP.to_owned(),
        litellm_params,
        model_info,
        top_level,
        String::new,
    )
    .unwrap()
}

pub(super) fn usage(tokens: i64, requests: i64) -> WindowUsage {
    WindowUsage { tokens, requests }
}

// The selection a route over GROUP asks the cost map for.
pub(super) fn selection<'request>(
    deployments: &'request [DeploymentSpec],
    input_tokens: i64,
) -> Selection<'request> {
    Selection {
        group: GROUP,
        deployments,
        input_tokens,
    }
}

pub(super) fn entry(id: &str, tokens: i64, requests: i64) -> Entry {
    Entry {
        deployment_id: id.to_owned(),
        usage: WindowUsage { tokens, requests },
    }
}

#[derive(Default)]
pub(super) struct FakeWindows {
    windows: Mutex<BTreeMap<(String, String, String), WindowUsage>>,
}

impl FakeWindows {
    pub(super) fn seed(&self, id: &str, minute: &str, usage: WindowUsage) {
        self.windows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert((GROUP.to_owned(), id.to_owned(), minute.to_owned()), usage);
    }
}

#[async_trait]
impl CostWindows for FakeWindows {
    fn usage(&self, model_group: &str, deployment_id: &str, minute: &str) -> WindowUsage {
        self.windows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(
                model_group.to_owned(),
                deployment_id.to_owned(),
                minute.to_owned(),
            ))
            .copied()
            .unwrap_or_default()
    }

    async fn record(&self, model_group: &str, deployment_id: &str, minute: &str, tokens: i64) {
        let mut windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = windows
            .entry((
                model_group.to_owned(),
                deployment_id.to_owned(),
                minute.to_owned(),
            ))
            .or_default();
        entry.tokens = entry.tokens.saturating_add(tokens);
        entry.requests = entry.requests.saturating_add(1);
    }
}

// A window store whose write never completes, so a cancelled observation stops at the await.
#[derive(Default)]
pub(super) struct PendingWindows;

#[async_trait]
impl CostWindows for PendingWindows {
    fn usage(&self, _model_group: &str, _deployment_id: &str, _minute: &str) -> WindowUsage {
        WindowUsage::default()
    }

    async fn record(&self, _model_group: &str, _deployment_id: &str, _minute: &str, _tokens: i64) {
        std::future::pending::<()>().await;
    }
}

pub(super) struct TestClock(Arc<AtomicU64>);

impl TestClock {
    pub(super) fn new(millis: u64) -> Self {
        Self(Arc::new(AtomicU64::new(millis)))
    }

    pub(super) fn set_millis(&self, millis: u64) {
        self.0.store(millis, Ordering::SeqCst);
    }

    pub(super) fn io_clock(&self) -> IoClock {
        let source = Arc::clone(&self.0);
        Arc::new(move || UNIX_EPOCH + Duration::from_millis(source.load(Ordering::SeqCst)))
    }

    pub(super) fn value_clock(&self) -> Clock {
        let source = Arc::clone(&self.0);
        Arc::new(move || Duration::from_millis(source.load(Ordering::SeqCst)).as_secs_f64())
    }
}

pub(super) fn now(millis: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis)
}

pub(super) struct NoDelta;

#[async_trait]
impl DeltaStore for NoDelta {
    async fn increment_pipeline(
        &self,
        _deltas: &[Delta],
    ) -> Result<Vec<Decimal>, RoutingStateError> {
        Ok(Vec::new())
    }
}

pub(super) fn value_store(clock: &TestClock) -> Arc<RoutingValueStore> {
    let settings = StoreSettings {
        max_size_in_memory: Some(4096),
        default_ttl_seconds: None,
        max_size_per_item: None,
        loaded_max_size_per_item_kib: 1024,
    };
    Arc::new(RoutingValueStore::new(settings, clock.value_clock()).unwrap())
}

// B304's counters over the same local store the windows read, with no shared store to flush to.
pub(super) fn real_router(
    clock: &TestClock,
) -> (CostRouter<WindowCounters<NoDelta>>, Arc<RoutingValueStore>) {
    let values = value_store(clock);
    let settings = Settings {
        sync_interval: None,
        default_sync_interval: Duration::from_secs(1),
        should_batch_redis_writes: false,
    };
    let counters = Arc::new(RouterCounters::new(
        settings,
        Arc::clone(&values),
        None::<Arc<NoDelta>>,
    ));
    (
        CostRouter::new(
            WindowCounters::new(counters, Arc::clone(&values)),
            clock.io_clock(),
        ),
        values,
    )
}

// Seed the pair of counters a success observation would have written.
pub(super) fn seed_window(
    values: &RoutingValueStore,
    group: &str,
    id: &str,
    minute: &str,
    tokens: i64,
    requests: i64,
) {
    let keys = window_keys(group, id, minute);
    for (key, value) in [(&keys.tokens, tokens), (&keys.requests, requests)] {
        values
            .set(
                &RouteKey::parse(key).unwrap(),
                &RouteValue::Spend(Decimal::from(value)),
                None,
            )
            .unwrap();
    }
}
