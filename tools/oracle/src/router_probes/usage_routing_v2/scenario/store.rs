use std::collections::BTreeMap;
use std::slice;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_router::routing_read_batch::local::{
    DEFAULT_LOCAL_TTL_SECONDS, DEFAULT_MAX_LAST_READS, DEFAULT_MAX_LOCAL_ENTRIES,
    DEFAULT_READ_EXPIRY_SECONDS, LocalTier, ReadTierSettings, TierState,
};
use betterlite_router::routing_read_batch::store::{
    BatchRead, BatchReadFuture, BatchReadStore, ReadWarning,
};
use betterlite_router::routing_value_store::{RouteKey, RouteValue, StoreError};
use betterlite_router::usage_routing_v2::error::KeyError;
use chrono::{DateTime, Utc};

use super::super::CounterInput;
use super::ScenarioError;

// the clock the local read tier and the window keys run on, which a step moves
pub(super) struct Clock {
    pub(super) seconds: Arc<Mutex<f64>>,
}

impl Clock {
    pub(super) fn new() -> Self {
        Self {
            seconds: Arc::new(Mutex::new(0.0)),
        }
    }
}

pub(super) struct StoreState {
    down: bool,
    reads: Vec<Vec<String>>,
}

// the shared store a selection reads: a counter per deployment, answered for whatever window key asks
pub(super) struct ScenarioStore {
    pairs: Vec<(String, String)>,
    counters: BTreeMap<String, CounterInput>,
    state: Mutex<StoreState>,
}

impl ScenarioStore {
    pub(super) fn new(
        pairs: Vec<(String, String)>,
        counters: BTreeMap<String, CounterInput>,
    ) -> Self {
        Self {
            pairs,
            counters,
            state: Mutex::new(StoreState {
                down: false,
                reads: Vec::new(),
            }),
        }
    }

    pub(super) fn set_down(&self, down: bool) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .down = down;
    }

    pub(super) fn reads(&self) -> Vec<Vec<String>> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reads
            .clone()
    }

    // the counter the deployment of this exact <id>:<model>:<tpm|rpm>:<HH-MM> key holds
    fn find(&self, raw: &str) -> Option<i64> {
        let (_, minute) = raw.rsplit_once(':')?;
        for (id, model) in &self.pairs {
            let Some(counter) = self.counters.get(id) else {
                continue;
            };
            for (field, value) in [("tpm", counter.tpm), ("rpm", counter.rpm)] {
                if format!("{id}:{model}:{field}:{minute}") == raw {
                    return value;
                }
            }
        }
        None
    }
}

impl BatchReadStore for ScenarioStore {
    type Value = RouteValue;

    fn batch_read<'store>(
        &'store self,
        keys: &'store [RouteKey],
    ) -> BatchReadFuture<'store, RouteValue> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state
                .reads
                .push(keys.iter().map(|key| key.as_ref().to_owned()).collect());
            let down = state.down;
            drop(state);
            if down {
                return BatchRead {
                    values: vec![None; keys.len()],
                    warning: Some(ReadWarning::Unavailable),
                };
            }
            BatchRead {
                values: keys
                    .iter()
                    .map(|key| self.find(key.as_ref()).map(RouteValue::Counter))
                    .collect(),
                warning: None,
            }
        })
    }
}

pub(super) fn new_tier(clock: Arc<dyn Fn() -> f64 + Send + Sync>) -> LocalTier<RouteValue> {
    LocalTier {
        settings: ReadTierSettings {
            backfill_ttl_seconds: DEFAULT_LOCAL_TTL_SECONDS,
            read_expiry_seconds: DEFAULT_READ_EXPIRY_SECONDS,
            max_entries: DEFAULT_MAX_LOCAL_ENTRIES,
            max_last_reads: DEFAULT_MAX_LAST_READS,
        },
        clock,
        state: Mutex::new(TierState::default()),
    }
}

pub(super) fn seed_tier(
    tier: &LocalTier<RouteValue>,
    local: &BTreeMap<String, i64>,
) -> Result<(), ScenarioError> {
    for (raw, value) in local {
        let key = RouteKey::parse(raw).map_err(|source: StoreError| ScenarioError::Key {
            source: KeyError::TooLong {
                deployment: raw.clone(),
                source,
            },
        })?;
        let fetched = BTreeMap::from([(key.clone(), RouteValue::Counter(*value))]);
        tier.apply(slice::from_ref(&key), &[None], &fetched);
    }
    Ok(())
}

pub(super) fn at(millis: u64) -> Result<DateTime<Utc>, ScenarioError> {
    match i64::try_from(millis)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
    {
        Some(instant) => Ok(instant),
        None => Err(ScenarioError::Time { millis }),
    }
}
