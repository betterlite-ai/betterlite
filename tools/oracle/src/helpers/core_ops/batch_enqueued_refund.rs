use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::local_ttl_cache::LocalTtlCache;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_enqueued_refund::store::EnqueuedLocalStore as _;
use betterlite_core::api::batch_enqueued_refund::{BatchEnqueuedRefundInput, EnqueuedTokenRefund};
use betterlite_core::api::batch_enqueued_reserve::store::{
    EnqueuedTokenCounters, LocalTokenCounters,
};
use betterlite_core::api::batch_enqueued_reserve::{
    BATCH_ENQUEUED_TTL_SECONDS, BatchEnqueuedTokenSettings,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};
use error::failure;

mod capture;
mod error;
mod local_store;
mod projection;
mod scripts;
mod tests;

const OWNER: &str = "oracle-worker";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    plan: BatchEnqueuedRefundInput,
    #[serde(default)]
    redis: Option<ScriptsInput>,
    #[serde(default)]
    memory: MemoryInput,
    #[serde(default)]
    now: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptsInput {
    #[serde(default)]
    refund: Vec<Step>,
    #[serde(default)]
    save: Vec<Step>,
    #[serde(default)]
    pop: Vec<PopStep>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Step {
    Done,
    Failure { message: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PopStep {
    Record { record: Value },
    Tombstone,
    Absent,
    Failure { message: String },
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MemoryInput {
    counters: BTreeMap<String, i64>,
    records: BTreeMap<String, String>,
    read: Vec<String>,
}

fn seeded_counters(memory: &MemoryInput) -> Result<LocalTokenCounters, Box<HelperError>> {
    let cache = LocalTtlCache::with_clock(None, None, None, 1024, Arc::new(|| 0.0_f64))
        .map_err(|error| failure(error.to_string()))?;
    let mut counters = LocalTokenCounters { cache };
    for (key, value) in &memory.counters {
        EnqueuedTokenCounters::set(&mut counters, key, *value, BATCH_ENQUEUED_TTL_SECONDS)
            .map_err(|error| failure(error.to_string()))?;
    }
    for (key, text) in &memory.records {
        counters
            .set_record(key, text, BATCH_ENQUEUED_TTL_SECONDS)
            .map_err(|error| failure(error.to_string()))?;
    }
    Ok(counters)
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        let events = Arc::new(Mutex::new(Vec::new()));
        let scripts = input
            .redis
            .clone()
            .map(|spec| scripts::ScriptedScripts::new(Arc::clone(&events), spec));
        let store = Arc::new(Mutex::new(local_store::RecordingStore {
            counters: seeded_counters(&input.memory)?,
            events: Arc::clone(&events),
        }));
        let now = input.now;
        let service = EnqueuedTokenRefund {
            clock: Arc::new(move || now),
            scripts,
            store: Arc::clone(&store),
            owner: OWNER.to_owned(),
            settings: BatchEnqueuedTokenSettings::default(),
            limits: ApiLimits::default(),
        };
        let guard = tracing::subscriber::set_default(
            tracing_subscriber::registry().with(capture::CaptureLayer(Arc::clone(&events))),
        );
        let result = runtime
            .block_on(service.run(&input.plan))
            .map_err(|error| failure(error.to_string()));
        drop(guard);
        let outcome = projection::outcome_value(&result?);
        let memory = projection::read_memory(&store, &input.memory.read)?;
        let captured = events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(json!({"outcome": outcome, "events": captured, "memory": memory}))
    }
}
