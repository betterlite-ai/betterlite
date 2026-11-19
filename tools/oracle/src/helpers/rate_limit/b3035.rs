mod fakes;
mod output;

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use tokio::sync::Mutex;

use betterlite_rate_limit::concurrency_gauges::ConcurrencyGauges;
use betterlite_rate_limit::concurrency_gauges::cache::GaugeCache;
use betterlite_rate_limit::fail_closed::FailClosedPolicy;
use betterlite_rate_limit::limiter_records::{
    LimiterClock, ParallelRequestGauge, ParallelSlotAcquisition, RateLimitDescriptor,
    RateLimitType, create_rate_limit_keys,
};
use betterlite_rate_limit::rate_limit_window_store::CounterScript;
use betterlite_rate_limit::request_stash::RequestStashSlot;
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use fakes::{FixedClock, PlannedScript, RecordingCache};
use output::{gauge_error, response_json};
use tokio::runtime::Builder;

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum CounterValueInput {
    Int(i64),
    Text(String),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum ReplyInput {
    Values {
        values: Vec<CounterValueInput>,
    },
    Failure {
        #[serde(rename = "failure")]
        _failure: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CacheValueInput {
    Int(i64),
    Float(f64),
    Text(String),
    Registry(BTreeMap<String, f64>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct GaugeCheckInput {
    descriptors: Vec<RateLimitDescriptor>,
    slot_id: String,
    read_only: bool,
    fail_closed: bool,
    now: u64,
    #[serde(default)]
    cache: BTreeMap<String, CacheValueInput>,
    replies: Option<Vec<ReplyInput>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReleaseInput {
    counter_keys: Vec<String>,
    slot_id: String,
    now: u64,
    #[serde(default)]
    cache: BTreeMap<String, CacheValueInput>,
    replies: Option<Vec<ReplyInput>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    GaugeCheck(GaugeCheckInput),
    Release(ReleaseInput),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::GaugeCheck(call) => gauge_check(call),
            Input::Release(call) => release_slot(call),
        }
    }
}

enum GaugeScripts {
    Check(Option<Arc<dyn CounterScript>>),
    Release(Option<Arc<dyn CounterScript>>),
}

fn concurrency_gauges(
    cache: Arc<dyn GaugeCache>,
    clock: Arc<dyn LimiterClock>,
    scripts: GaugeScripts,
    fail_closed: bool,
) -> ConcurrencyGauges {
    let (acquire_script, count_script, release_script) = match scripts {
        GaugeScripts::Check(script) => (script.clone(), script, None),
        GaugeScripts::Release(script) => (None, None, script),
    };
    ConcurrencyGauges {
        cache,
        clock,
        acquire_script,
        release_script,
        count_script,
        release_batch: None,
        fail_closed: FailClosedPolicy {
            enabled: fail_closed,
        },
        check_and_increment_lock: Mutex::new(()),
    }
}

fn clock_at(now: u64) -> Result<FixedClock, Box<HelperError>> {
    FixedClock::at(now)
        .ok_or("now is too large")
        .or_value_error()
}

fn shared_script(handle: Option<&Arc<PlannedScript>>) -> Option<Arc<dyn CounterScript>> {
    let planned = handle?;
    let script: Arc<dyn CounterScript> = Arc::<PlannedScript>::clone(planned);
    Some(script)
}

fn gauge_check(call: &GaugeCheckInput) -> HelperResult {
    let cache = Arc::new(RecordingCache::new(&call.cache));
    let handle = call
        .replies
        .as_ref()
        .map(|plan| Arc::new(PlannedScript::new(plan.clone())));
    let shared: Arc<dyn GaugeCache> = Arc::<RecordingCache>::clone(&cache);
    let clock: Arc<dyn LimiterClock> = Arc::new(clock_at(call.now)?);
    let limiter = concurrency_gauges(
        shared,
        clock,
        GaugeScripts::Check(shared_script(handle.as_ref())),
        call.fail_closed,
    );
    let gauges = requested_gauges(&call.descriptors);
    let runtime = Builder::new_current_thread().build()?;
    let response = runtime
        .block_on(limiter.check_parallel_request_gauges(&gauges, &call.slot_id, call.read_only))
        .map_err(|error| gauge_error(&error))?;
    let (reads, batch_reads, writes) = cache.snapshot();
    Ok(json!({
        "response": response_json(&response),
        "calls": handle.as_deref().map_or_else(Vec::new, PlannedScript::calls),
        "reads": reads,
        "batch_reads": batch_reads,
        "writes": writes,
    }))
}

fn release_slot(call: &ReleaseInput) -> HelperResult {
    let cache = Arc::new(RecordingCache::new(&call.cache));
    let handle = call
        .replies
        .as_ref()
        .map(|plan| Arc::new(PlannedScript::new(plan.clone())));
    let shared: Arc<dyn GaugeCache> = Arc::<RecordingCache>::clone(&cache);
    let clock: Arc<dyn LimiterClock> = Arc::new(clock_at(call.now)?);
    let limiter = concurrency_gauges(
        shared,
        clock,
        GaugeScripts::Release(shared_script(handle.as_ref())),
        false,
    );
    let slot = RequestStashSlot::default();
    let stash = slot.get_or_create();
    let acquisition = ParallelSlotAcquisition {
        slot_id: call.slot_id.clone(),
        counter_keys: call.counter_keys.clone(),
    };
    stash
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .parallel_slot = Some(acquisition);
    let runtime = Builder::new_current_thread().build()?;
    runtime.block_on(limiter.release_stashed_parallel_slot(Some(stash.as_ref()), false));
    let (reads, batch_reads, writes) = cache.snapshot();
    Ok(json!({
        "response": null,
        "calls": handle.as_deref().map_or_else(Vec::new, PlannedScript::calls),
        "reads": reads,
        "batch_reads": batch_reads,
        "writes": writes,
    }))
}

fn requested_gauges(descriptors: &[RateLimitDescriptor]) -> Vec<ParallelRequestGauge> {
    let mut gauges = Vec::new();
    for descriptor in descriptors {
        let Some(rate_limit) = descriptor.rate_limit.as_ref() else {
            continue;
        };
        let Some(limit) = rate_limit.max_parallel_requests else {
            continue;
        };
        let counter_key = create_rate_limit_keys(
            &descriptor.key,
            &descriptor.value,
            RateLimitType::MaxParallelRequests,
        );
        gauges.push(ParallelRequestGauge {
            counter_key,
            limit,
            descriptor_key: descriptor.key.clone(),
        });
    }
    gauges
}
