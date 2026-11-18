use std::collections::BTreeMap;
use std::convert::Infallible;
use std::slice;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, Utc};
use serde_json::Number;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::routing_read_batch::local::{LocalTier, ReadTierSettings, TierState};
use crate::routing_read_batch::store::{BatchRead, BatchReadFuture, BatchReadStore, ReadWarning};
use crate::routing_read_batch::{ReadTiers, RoutingReadKeys, UsageReadKeys};
use crate::routing_value_store::{RouteKey, RouteValue};
use crate::usage_routing_v2::selection::Selection;
use crate::usage_routing_v2::service::UsageRoutingV2;

pub(super) const GROUP: &str = "gpt-4o";

pub(super) fn at(millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(millis).expect("the test time is representable")
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

// A deployment of GROUP with the limits and extra keys a case names at each of the three levels.
pub(super) fn deployment(
    id: &str,
    params: &[(&str, Json)],
    info: &[(&str, Json)],
    extra: &[(&str, Json)],
) -> DeploymentSpec {
    let mut litellm_params = JsonObject::new();
    litellm_params.insert("model".to_owned(), text(GROUP));
    for (name, value) in params {
        litellm_params.insert((*name).to_owned(), value.clone());
    }
    let mut model_info = JsonObject::new();
    model_info.insert("id".to_owned(), text(id));
    for (name, value) in info {
        model_info.insert((*name).to_owned(), value.clone());
    }
    let mut top = JsonObject::new();
    for (name, value) in extra {
        top.insert((*name).to_owned(), value.clone());
    }
    DeploymentSpec::new(
        GROUP.to_owned(),
        litellm_params,
        model_info,
        top,
        String::new,
    )
    .expect("the test deployment is valid")
}

pub(super) fn plain(id: &str) -> DeploymentSpec {
    deployment(id, &[], &[], &[])
}

pub(super) fn counter(value: i64) -> RouteValue {
    RouteValue::Counter(value)
}

pub(super) fn route_key(raw: &str) -> RouteKey {
    RouteKey::parse(raw).expect("the test key is within the store's limit")
}

pub(super) fn tpm_window(id: &str) -> String {
    format!("{id}:{GROUP}:tpm:00-00")
}

pub(super) fn rpm_window(id: &str) -> String {
    format!("{id}:{GROUP}:rpm:00-00")
}

// The one selection a case makes through the service and the request's token estimate.
pub(super) fn service(deployments: &[DeploymentSpec], tokens: i64) -> UsageRoutingV2<'_> {
    UsageRoutingV2 {
        group: GROUP,
        deployments,
        input_tokens: tokens,
        now: at(0),
    }
}

// The usage keys a request declares for the deployments a case names.
pub(super) fn reads(names: &[&str]) -> RoutingReadKeys {
    RoutingReadKeys {
        cooldown: Vec::new(),
        usage: Some(UsageReadKeys {
            tpm: names.iter().map(|id| route_key(&tpm_window(id))).collect(),
            rpm: names.iter().map(|id| route_key(&rpm_window(id))).collect(),
        }),
    }
}

// The values one read returns, as the strategy sees them.
pub(super) fn values(values: &[Option<i64>]) -> Vec<Option<RouteValue>> {
    values.iter().map(|value| value.map(counter)).collect()
}

// The one selection a case makes, over the pool and whatever a read returned for it.
pub(super) fn selection<'pool>(
    deployments: &'pool [DeploymentSpec],
    tpm: Option<&'pool [Option<RouteValue>]>,
    rpm: Option<&'pool [Option<RouteValue>]>,
    input_tokens: i64,
) -> Selection<'pool> {
    Selection {
        group: GROUP,
        deployments,
        input_tokens,
        tpm,
        rpm,
    }
}

// A sampled rng that answers the same u32 draw each time: 1 is the first index every range accepts and
// u32::MAX is its last.
pub(super) struct FixedRng(pub(super) u32);

impl rand::TryRng for FixedRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        Ok(self.0)
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        Ok(u64::from(self.0))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        for slot in dst.iter_mut() {
            *slot = 0;
        }
        Ok(())
    }
}

pub(super) fn first() -> FixedRng {
    FixedRng(1)
}

pub(super) fn last() -> FixedRng {
    FixedRng(u32::MAX)
}

// The shared store a selection reads: the values it holds and every batch it was asked for.
pub(super) struct FakeStore {
    values: BTreeMap<RouteKey, RouteValue>,
    down: bool,
    hang: bool,
    reads: Mutex<Vec<Vec<String>>>,
}

impl FakeStore {
    pub(super) fn new(entries: &[(&str, i64)]) -> Self {
        Self {
            values: entries
                .iter()
                .map(|(raw, value)| (route_key(raw), counter(*value)))
                .collect(),
            down: false,
            hang: false,
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn go_down(&mut self) {
        self.down = true;
    }

    pub(super) fn hang(&mut self) {
        self.hang = true;
    }

    pub(super) fn go_live(&mut self) {
        self.hang = false;
    }

    pub(super) fn set(&mut self, raw: &str, value: i64) {
        self.values.insert(route_key(raw), counter(value));
    }

    pub(super) fn reads(&self) -> Vec<Vec<String>> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl BatchReadStore for FakeStore {
    type Value = RouteValue;

    fn batch_read<'store>(
        &'store self,
        keys: &'store [RouteKey],
    ) -> BatchReadFuture<'store, RouteValue> {
        Box::pin(async move {
            self.reads
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(keys.iter().map(|key| key.as_ref().to_owned()).collect());
            if self.hang {
                std::future::pending::<()>().await;
            }
            if self.down {
                return BatchRead {
                    values: vec![None; keys.len()],
                    warning: Some(ReadWarning::Unavailable),
                };
            }
            BatchRead {
                values: keys
                    .iter()
                    .map(|key| self.values.get(key).cloned())
                    .collect(),
                warning: None,
            }
        })
    }
}

// The tier clock the read expiry runs on, which a case moves between requests.
#[derive(Clone)]
pub(super) struct TestClock(Arc<Mutex<f64>>);

impl TestClock {
    pub(super) fn new() -> Self {
        Self(Arc::new(Mutex::new(0.0)))
    }

    pub(super) fn set(&self, seconds: f64) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = seconds;
    }

    fn source(&self) -> Arc<dyn Fn() -> f64 + Send + Sync> {
        let shared = Arc::clone(&self.0);
        Arc::new(move || *shared.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

pub(super) fn usage_tier(clock: &TestClock) -> LocalTier<RouteValue> {
    LocalTier {
        settings: ReadTierSettings {
            backfill_ttl_seconds: 600.0,
            read_expiry_seconds: 10.0,
            max_entries: 200,
            max_last_reads: 1_000,
        },
        clock: clock.source(),
        state: Mutex::new(TierState::default()),
    }
}

pub(super) fn tiers(clock: &TestClock) -> ReadTiers<RouteValue> {
    ReadTiers {
        cooldown: Arc::new(usage_tier(clock)),
        usage: Some(Arc::new(usage_tier(clock))),
    }
}

pub(super) fn seed(tier: &LocalTier<RouteValue>, entries: &[(&str, i64)]) {
    for (raw, value) in entries {
        let key = route_key(raw);
        let fetched = BTreeMap::from([(key.clone(), counter(*value))]);
        tier.backfill(slice::from_ref(&key), &fetched);
    }
}
