use std::collections::BTreeMap;
use std::slice;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_router::routing_read_batch::local::{LocalTier, ReadTierSettings, TierState};
use betterlite_router::routing_read_batch::store::{
    BatchRead, BatchReadFuture, BatchReadStore, ReadWarning,
};
use betterlite_router::routing_read_batch::{
    ReadTiers, RequestReads, RoutingPrefetch, RoutingReadBatch, RoutingReadKeys,
    RoutingReadSession, UsageReadKeys,
};
use betterlite_router::routing_value_store::{RouteKey, RouteValue, StoreError};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use tokio::runtime::Builder;

use super::{Input, Output, ScenarioError, Step, StepKind, StepOutput, UsageInput, UsageOutput};

const DEFAULT_READ_EXPIRY_MILLIS: u64 = 10_000;

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
        let seconds = millis_to_seconds(millis)?;
        *self.seconds.lock().unwrap_or_else(PoisonError::into_inner) = seconds;
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
    usage_keys: Option<UsageReadKeys>,
    clock: Clock,
    tiers: ReadTiers<RouteValue>,
    store: ScenarioStore,
    requests: Vec<RequestReads<RouteValue>>,
}

impl Scenario {
    fn new(input: &Input) -> Result<Self, ScenarioError> {
        let cooldown_keys = parse_keys(&input.cooldown.keys)?;
        let usage_keys = match &input.usage {
            Some(usage) => Some(usage_keys(usage)?),
            None => None,
        };
        let clock = Clock {
            seconds: Arc::new(Mutex::new(0.0_f64)),
        };
        Ok(Self {
            cooldown_keys,
            usage_keys,
            tiers: build_tiers(input, &clock)?,
            clock,
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
        let mut usage = None;
        let mut prefetched_usage = None;
        match step.kind {
            StepKind::Arm => armed = Some(self.arm(step)?),
            StepKind::Read => {
                let outcome = self.read(step).await?;
                active = Some(outcome.active);
                cooldown = outcome.cooldown;
                usage = outcome.usage;
                prefetched_usage = outcome.prefetched_usage;
            }
            StepKind::Down => self.store.set_down(step.down),
        }
        let cooldown_local = self.tiers.cooldown.batch_get(&self.cooldown_keys);
        let usage_local = match (&self.usage_keys, self.tiers.usage.as_ref()) {
            (Some(keys), Some(tier)) => tier.batch_get(&keys.combined()),
            _ => Vec::new(),
        };
        Ok(StepOutput {
            kind: step.kind,
            armed,
            active,
            cooldown,
            usage,
            prefetched_usage,
            cooldown_local,
            usage_local,
        })
    }

    fn arm(&mut self, step: &Step) -> Result<bool, ScenarioError> {
        let usage = self.select_usage(step)?;
        let keys = RoutingReadKeys {
            cooldown: self.cooldown_keys.clone(),
            usage,
        };
        let request = self
            .requests
            .get_mut(step.request)
            .ok_or(ScenarioError::UnknownRequest {
                request: step.request,
            })?;
        Ok(RoutingPrefetch::arm(&keys, &self.tiers, true, request))
    }

    async fn read(&mut self, step: &Step) -> Result<ReadOutcome, ScenarioError> {
        let usage = self.select_usage(step)?;
        let armed = self
            .requests
            .get(step.request)
            .ok_or(ScenarioError::UnknownRequest {
                request: step.request,
            })?
            .prefetch
            .is_some();
        let Some(mut batch) = RoutingReadBatch::for_strategy(usage, armed) else {
            return Ok(ReadOutcome::inactive());
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
        let prefetched_usage = self.usage_output(&batch);
        Ok(ReadOutcome {
            active: true,
            cooldown: Some(results.cooldown),
            usage: results.usage,
            prefetched_usage,
        })
    }

    fn select_usage(&self, step: &Step) -> Result<Option<UsageReadKeys>, ScenarioError> {
        if !step.usage_v2 {
            return Ok(None);
        }
        self.usage_keys
            .clone()
            .map(Some)
            .ok_or(ScenarioError::MissingUsageKeys)
    }

    fn usage_output(&self, batch: &RoutingReadBatch<RouteValue>) -> Option<UsageOutput> {
        let prefetched = batch.prefetched_usage.as_ref()?;
        let keys = self.usage_keys.as_ref()?.combined();
        Some(UsageOutput {
            covers: prefetched.covers(&keys),
            values: prefetched.values_for(&keys),
        })
    }
}

struct ReadOutcome {
    active: bool,
    cooldown: Option<Vec<Option<RouteValue>>>,
    usage: Option<Vec<Option<RouteValue>>>,
    prefetched_usage: Option<UsageOutput>,
}

impl ReadOutcome {
    const fn inactive() -> Self {
        Self {
            active: false,
            cooldown: None,
            usage: None,
            prefetched_usage: None,
        }
    }
}

fn build_tiers(input: &Input, clock: &Clock) -> Result<ReadTiers<RouteValue>, ScenarioError> {
    let backfill_ttl_seconds = match input.backfill_ttl_millis {
        Some(millis) => Some(millis_to_seconds(millis)?),
        None => None,
    };
    let settings = ReadTierSettings {
        backfill_ttl_seconds,
        read_expiry_seconds: millis_to_seconds(
            input
                .read_expiry_millis
                .unwrap_or(DEFAULT_READ_EXPIRY_MILLIS),
        )?,
    };
    let cooldown = new_tier(settings, clock);
    seed_tier(&cooldown, &input.cooldown.local)?;
    let usage = match &input.usage {
        Some(entry) => {
            let tier = new_tier(settings, clock);
            seed_tier(&tier, &entry.local)?;
            Some(tier)
        }
        None => None,
    };
    Ok(ReadTiers {
        cooldown: Arc::new(cooldown),
        usage: usage.map(Arc::new),
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

fn usage_keys(usage: &UsageInput) -> Result<UsageReadKeys, ScenarioError> {
    Ok(UsageReadKeys {
        tpm: parse_keys(&usage.tpm)?,
        rpm: parse_keys(&usage.rpm)?,
    })
}

fn parse_keys(raw: &[String]) -> Result<Vec<RouteKey>, ScenarioError> {
    let mut keys = Vec::with_capacity(raw.len());
    for text in raw {
        keys.push(parse_key(text)?);
    }
    Ok(keys)
}

fn parse_key(raw: &str) -> Result<RouteKey, ScenarioError> {
    RouteKey::parse(raw).map_err(|source: StoreError| ScenarioError::InvalidKey {
        key: raw.to_owned(),
        source,
    })
}

fn millis_to_seconds(millis: u64) -> Result<f64, ScenarioError> {
    let seconds = Decimal::from(millis)
        .checked_div(Decimal::ONE_THOUSAND)
        .and_then(|value| value.to_f64());
    seconds.ok_or(ScenarioError::TimeOverflow { millis })
}
