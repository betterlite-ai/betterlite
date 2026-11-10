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
use betterlite_router::routing_read_batch::{
    ReadTiers, RequestReads, RoutingPrefetch, RoutingReadBatch, RoutingReadKeys, RoutingReadSession,
};
use betterlite_router::routing_value_store::{RouteKey, RouteValue, StoreError};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use tokio::runtime::Builder;

use super::{Input, Output, ScenarioError, Step, StepKind, StepOutput};

pub(crate) fn run(input: &Input) -> Result<Output, ScenarioError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| ScenarioError::Runtime { source })?;
    runtime.block_on(run_inner(input))
}

async fn run_inner(input: &Input) -> Result<Output, ScenarioError> {
    let mut scenario = Scenario::new(input)?;
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        steps.push(scenario.run_step(step).await?);
    }
    Ok(Output {
        steps,
        reads: scenario.store.reads(),
    })
}

struct Clock {
    seconds: Arc<Mutex<f64>>,
}

impl Clock {
    fn set_millis(&self, millis: u64) -> Result<(), ScenarioError> {
        *self.seconds.lock().unwrap_or_else(PoisonError::into_inner) = seconds(millis)?;
        Ok(())
    }
}

struct StoreState {
    values: BTreeMap<RouteKey, RouteValue>,
    down: bool,
    reads: Vec<Vec<RouteKey>>,
}

struct ScenarioStore {
    state: Mutex<StoreState>,
}

impl ScenarioStore {
    fn new(remote: &BTreeMap<String, RouteValue>) -> Result<Self, ScenarioError> {
        let mut values = BTreeMap::new();
        for (raw, value) in remote {
            values.insert(parse_key(raw)?, value.clone());
        }
        Ok(Self {
            state: Mutex::new(StoreState {
                values,
                down: false,
                reads: Vec::new(),
            }),
        })
    }

    fn set_down(&self, down: bool) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .down = down;
    }

    fn reads(&self) -> Vec<Vec<String>> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reads
            .iter()
            .map(|keys| keys.iter().map(|key| key.as_ref().to_owned()).collect())
            .collect()
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
            state.reads.push(keys.to_vec());
            let down = state.down;
            let values = if down {
                vec![None; keys.len()]
            } else {
                keys.iter()
                    .map(|key| state.values.get(key).cloned())
                    .collect()
            };
            drop(state);
            BatchRead {
                values,
                warning: down.then_some(ReadWarning::Unavailable),
            }
        })
    }
}

struct Scenario {
    cooldown_keys: Vec<RouteKey>,
    clock: Clock,
    tiers: ReadTiers<RouteValue>,
    store: ScenarioStore,
    requests: Vec<RequestReads<RouteValue>>,
}

impl Scenario {
    fn new(input: &Input) -> Result<Self, ScenarioError> {
        let mut cooldown_keys = Vec::with_capacity(input.cooldown.keys.len());
        for text in &input.cooldown.keys {
            cooldown_keys.push(parse_key(text)?);
        }
        let clock = Clock {
            seconds: Arc::new(Mutex::new(0.0_f64)),
        };
        let cooldown = new_tier(settings(input)?, &clock);
        seed_tier(&cooldown, &input.cooldown.local)?;
        Ok(Self {
            cooldown_keys,
            clock,
            tiers: ReadTiers {
                cooldown: Arc::new(cooldown),
                usage: None,
            },
            store: ScenarioStore::new(&input.remote)?,
            requests: vec![
                RequestReads { prefetch: None },
                RequestReads { prefetch: None },
            ],
        })
    }

    async fn run_step(&mut self, step: &Step) -> Result<StepOutput, ScenarioError> {
        self.clock.set_millis(step.now_millis)?;
        let mut armed = None;
        let mut active = None;
        let mut cooldown = None;
        match step.kind {
            StepKind::Arm => armed = Some(self.arm(step)?),
            StepKind::Read => {
                let outcome = self.read(step).await?;
                active = Some(outcome.0);
                cooldown = outcome.1;
            }
            StepKind::Down => self.store.set_down(step.down),
        }
        Ok(StepOutput {
            kind: step.kind,
            armed,
            active,
            cooldown,
            cooldown_local: self.tiers.cooldown.batch_get(&self.cooldown_keys),
        })
    }

    fn arm(&mut self, step: &Step) -> Result<bool, ScenarioError> {
        let keys = RoutingReadKeys {
            cooldown: self.cooldown_keys.clone(),
            usage: None,
        };
        let request = self
            .requests
            .get_mut(step.request)
            .ok_or(ScenarioError::UnknownRequest {
                request: step.request,
            })?;
        Ok(RoutingPrefetch::arm(&keys, &self.tiers, true, request))
    }

    async fn read(
        &mut self,
        step: &Step,
    ) -> Result<(bool, Option<Vec<Option<RouteValue>>>), ScenarioError> {
        let armed = self
            .requests
            .get(step.request)
            .ok_or(ScenarioError::UnknownRequest {
                request: step.request,
            })?
            .prefetch
            .is_some();
        let Some(mut batch) = RoutingReadBatch::for_strategy(None, armed) else {
            return Ok((false, None));
        };
        let request = self
            .requests
            .get_mut(step.request)
            .ok_or(ScenarioError::UnknownRequest {
                request: step.request,
            })?;
        let results = {
            let mut session = RoutingReadSession {
                store: &self.store,
                tiers: &self.tiers,
                request,
            };
            batch
                .read_routing_state(&mut session, &self.cooldown_keys)
                .await
        };
        Ok((true, Some(results.cooldown)))
    }
}

fn settings(input: &Input) -> Result<ReadTierSettings, ScenarioError> {
    let backfill_ttl_seconds = match input.backfill_ttl_millis {
        Some(millis) => seconds(millis)?,
        None => DEFAULT_LOCAL_TTL_SECONDS,
    };
    let read_expiry_seconds = match input.read_expiry_millis {
        Some(millis) => seconds(millis)?,
        None => DEFAULT_READ_EXPIRY_SECONDS,
    };
    Ok(ReadTierSettings {
        backfill_ttl_seconds,
        read_expiry_seconds,
        max_entries: DEFAULT_MAX_LOCAL_ENTRIES,
        max_last_reads: DEFAULT_MAX_LAST_READS,
    })
}

fn new_tier(settings: ReadTierSettings, clock: &Clock) -> LocalTier<RouteValue> {
    let shared = Arc::clone(&clock.seconds);
    LocalTier {
        settings,
        clock: Arc::new(move || *shared.lock().unwrap_or_else(PoisonError::into_inner)),
        state: Mutex::new(TierState::default()),
    }
}

fn seed_tier(
    tier: &LocalTier<RouteValue>,
    local: &BTreeMap<String, RouteValue>,
) -> Result<(), ScenarioError> {
    for (raw, value) in local {
        let key = parse_key(raw)?;
        let fetched = BTreeMap::from([(key.clone(), value.clone())]);
        tier.apply(slice::from_ref(&key), &[None], &fetched);
    }
    Ok(())
}

fn parse_key(raw: &str) -> Result<RouteKey, ScenarioError> {
    RouteKey::parse(raw).map_err(|source: StoreError| ScenarioError::InvalidKey {
        key: raw.to_owned(),
        source,
    })
}

fn seconds(millis: u64) -> Result<f64, ScenarioError> {
    let Some(quotient) = Decimal::from(millis).checked_div(Decimal::from(1_000_i64)) else {
        return Err(ScenarioError::TimeOverflow { millis });
    };
    quotient
        .to_f64()
        .ok_or(ScenarioError::TimeOverflow { millis })
}
