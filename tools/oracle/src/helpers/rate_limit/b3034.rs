mod fakes;
mod output;

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use betterlite_rate_limit::atomic_counter::payload::IncrementAmounts;
use betterlite_rate_limit::fail_closed::FailClosedPolicy;
use betterlite_rate_limit::limiter_records::{
    CacheCounterValue, RateLimitDescriptor, RateLimitResponse,
};
use betterlite_rate_limit::rate_limit_window_store::{
    CounterScript, RequestBatchSource, WindowStore,
};
use betterlite_rate_limit::settlement_and_rollback::AtomicSettlement;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::HelperError;

use fakes::{FixedClock, PlannedBatch, PlannedBatchSource, PlannedScript, RecordingCache};
use output::{response_json, settlement_error};

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum CounterValueInput {
    Int(i64),
    Text(String),
}

impl CounterValueInput {
    fn counter_value(&self) -> CacheCounterValue {
        match self {
            Self::Int(number) => CacheCounterValue::Int(*number),
            Self::Text(text) => CacheCounterValue::Text(text.clone()),
        }
    }
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
pub(crate) struct Input {
    descriptors: Vec<RateLimitDescriptor>,
    increments: Vec<IncrementAmounts>,
    window_size: i64,
    now: u64,
    pipelined: bool,
    fail_closed: bool,
    #[serde(default)]
    cache: BTreeMap<String, CounterValueInput>,
    replies: Vec<ReplyInput>,
    refund_replies: Vec<ReplyInput>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let cache = Arc::new(RecordingCache::new(&input.cache));
        let main_script = Arc::new(PlannedScript::new(input.replies.clone()));
        let refund_script = Arc::new(PlannedScript::new(input.refund_replies.clone()));
        let batch = input
            .pipelined
            .then(|| Arc::new(PlannedBatch::new(input.replies.clone())));
        let store = window_store(Arc::clone(&cache), batch.as_ref());
        let clock = FixedClock::at(input.now);
        let script: Arc<dyn CounterScript> = Arc::<PlannedScript>::clone(&main_script);
        let refunds: Arc<dyn CounterScript> = Arc::<PlannedScript>::clone(&refund_script);
        let response = settle(input, &store, &clock, script, refunds)?;
        let calls = match &batch {
            Some(batch) => batch.calls(),
            None => main_script.calls(),
        };
        Ok(json!({
            "response": response_json(&response),
            "calls": calls,
            "refunds": refund_script.calls(),
            "reads": cache.reads(),
            "writes": cache.writes(),
        }))
    }
}

fn window_store(cache: Arc<RecordingCache>, batch: Option<&Arc<PlannedBatch>>) -> WindowStore {
    WindowStore {
        cache,
        rate_limiter_script: None,
        counter_read_script: None,
        request_batch: batch_source(batch),
        check_and_increment_lock: Mutex::new(()),
    }
}

fn batch_source(batch: Option<&Arc<PlannedBatch>>) -> Option<Arc<dyn RequestBatchSource>> {
    let planned = batch?;
    let source = PlannedBatchSource {
        batch: Arc::clone(planned),
    };
    let shared: Arc<dyn RequestBatchSource> = Arc::new(source);
    Some(shared)
}

fn settle(
    input: &Input,
    store: &WindowStore,
    clock: &FixedClock,
    script: Arc<dyn CounterScript>,
    refund_script: Arc<dyn CounterScript>,
) -> Result<RateLimitResponse, Box<HelperError>> {
    let settlement = AtomicSettlement {
        windows: store,
        clock,
        script: Some(script),
        refund_script: Some(refund_script),
        fail_closed: FailClosedPolicy {
            enabled: input.fail_closed,
        },
        default_window_size: input.window_size,
    };
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    runtime
        .block_on(settlement.atomic_check_and_increment_by_n(&input.descriptors, &input.increments))
        .map_err(settlement_error)
}
