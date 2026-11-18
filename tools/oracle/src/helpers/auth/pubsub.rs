mod fakes;
mod warnings;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use betterlite_auth::auth_cache::gateway_cache;
use betterlite_auth::auth_pubsub::message::Invalidation;
use betterlite_auth::auth_pubsub::publish::{
    MAX_PENDING_PUBLISHES, PublishTasks, evict_and_broadcast, publish_invalidation,
};
use betterlite_auth::auth_pubsub::subscriber::{CacheInvalidationSubscriber, Session};
use betterlite_cache::backend::BackendError;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio::task::yield_now;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use self::fakes::{Clock, Endpoint, Eviction, PublishMode, Step};
use self::warnings::Warnings;

const CLOCK_NOW: f64 = 1000.0_f64;
const SETTLE: usize = 1000;
const SETTINGS: DualCacheSettings = DualCacheSettings {
    default_in_memory_ttl: None,
    default_redis_ttl: None,
    batch_cache_expiry: None,
    max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
    redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
};

#[derive(Default)]
struct Journal {
    events: Vec<String>,
    publications: Vec<Value>,
    warnings: Vec<Value>,
    caches: BTreeMap<String, Value>,
}

type Shared = Arc<Mutex<Journal>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Deserialize)]
pub(crate) struct Input {
    evict: Option<EvictCall>,
    capacity: Option<bool>,
    subscriber: Option<SubscriberCall>,
}

#[derive(Deserialize)]
pub(crate) struct EvictCall {
    keys: Vec<String>,
    #[serde(default)]
    fail_publish: bool,
}

#[derive(Deserialize)]
pub(crate) struct SubscriberCall {
    sessions: Vec<Vec<StepInput>>,
    probe_keys: Option<Vec<String>>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub(crate) enum StepInput {
    Block,
    Fail,
    Message { message: String },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let journal = Arc::new(Mutex::new(Journal::default()));
        let subscriber = registry().with(Warnings(Arc::clone(&journal)));
        let outcome = tracing::subscriber::with_default(subscriber, || {
            runtime.block_on(run_sections(input, Arc::clone(&journal)))
        });
        outcome?;
        Ok(journal_value(&journal))
    }
}

async fn run_sections(input: &Input, journal: Shared) -> Result<(), Box<HelperError>> {
    if let Some(call) = &input.evict {
        evict_section(call, &journal).await;
    }
    if input.capacity.unwrap_or(false) {
        capacity_section(&journal).await;
    }
    if let Some(call) = &input.subscriber {
        subscriber_section(call, &journal).await?;
    }
    Ok(())
}

async fn evict_section(call: &EvictCall, journal: &Shared) {
    let publish = if call.fail_publish {
        PublishMode::Fail
    } else {
        PublishMode::Accept
    };
    let endpoint = Arc::new(Endpoint::new(Arc::clone(journal), Vec::new(), publish));
    let evidence = Eviction {
        journal: Arc::clone(journal),
    };
    evict_and_broadcast(
        &call.keys,
        &evidence,
        &PublishTasks::default(),
        Some(&endpoint),
    )
    .await;
}

async fn capacity_section(journal: &Shared) {
    let endpoint = Arc::new(Endpoint::new(
        Arc::clone(journal),
        Vec::new(),
        PublishMode::Block,
    ));
    let tasks = PublishTasks::default();
    for index in 0..=MAX_PENDING_PUBLISHES {
        let invalidation = Invalidation {
            cache_key: format!("cap:{index}"),
            new_value: None,
            ttl: None,
        };
        publish_invalidation(&tasks, Some(&endpoint), &invalidation).await;
    }
}

async fn subscriber_section(
    call: &SubscriberCall,
    journal: &Shared,
) -> Result<(), Box<HelperError>> {
    let now: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| CLOCK_NOW);
    let ordinary = Arc::new(LocalMemoryOps::new(1024, Arc::clone(&now)).or_value_error()?);
    let key_objects = Arc::new(LocalMemoryOps::new(1024, Arc::clone(&now)).or_value_error()?);
    let additional = Arc::new(LocalMemoryOps::new(1024, now).or_value_error()?);
    let probe_keys: &[String] = call.probe_keys.as_deref().unwrap_or_default();
    seed(&ordinary, probe_keys, 2.0_f64)?;
    seed(&key_objects, probe_keys, 1.0_f64)?;
    seed(&additional, probe_keys, 3.0_f64)?;
    let sessions = call
        .sessions
        .iter()
        .map(|steps| session_plan(steps))
        .collect();
    let subscriber = Arc::new(CacheInvalidationSubscriber {
        endpoint: Arc::new(Endpoint::new(
            Arc::clone(journal),
            sessions,
            PublishMode::Accept,
        )),
        clock: Clock,
        user_cache: Arc::new(gateway_cache(
            Arc::clone(&ordinary),
            Arc::clone(&key_objects),
            None,
            SETTINGS,
            45.0_f64,
        )),
        additional: vec![Arc::clone(&additional)],
        session: Mutex::new(Session::default()),
    });
    subscriber.start();
    let expected = call
        .sessions
        .iter()
        .map(|steps| steps.len().max(1))
        .sum::<usize>();
    settle(journal, "poll:", expected).await?;
    subscriber.stop().await;
    report_caches(
        journal,
        probe_keys,
        [
            ("ordinary", ordinary.as_ref()),
            ("key_objects", key_objects.as_ref()),
            ("additional:0", additional.as_ref()),
        ],
    )
}

fn session_plan(steps: &[StepInput]) -> Vec<Step> {
    steps
        .iter()
        .map(|step| match step {
            StepInput::Block => Step::Block,
            StepInput::Fail => Step::Fail,
            StepInput::Message { message } => Step::Message(message.as_bytes().to_vec()),
        })
        .collect()
}

fn seed(memory: &LocalMemoryOps, keys: &[String], value: f64) -> Result<(), Box<HelperError>> {
    for key in keys {
        memory
            .set_cache(key, CacheValue::Float(value), None)
            .or_value_error()?;
    }
    Ok(())
}

fn report_caches(
    journal: &Shared,
    keys: &[String],
    memories: [(&str, &LocalMemoryOps); 3],
) -> Result<(), Box<HelperError>> {
    let mut caches = BTreeMap::new();
    for (label, memory) in memories {
        caches.insert(label.to_owned(), states(memory, keys)?);
    }
    lock(journal).caches = caches;
    Ok(())
}

fn states(memory: &LocalMemoryOps, keys: &[String]) -> Result<Value, Box<HelperError>> {
    let mut states = BTreeMap::new();
    for key in keys {
        states.insert(key.clone(), state(memory, key).or_value_error()?);
    }
    Ok(Value::Object(states.into_iter().collect()))
}

fn state(memory: &LocalMemoryOps, key: &str) -> Result<Value, BackendError> {
    let value = match memory.get_cache(key)? {
        Some(CacheValue::Float(number)) => json!(number),
        Some(_) | None => Value::Null,
    };
    let expiry = memory
        .ttl_entries()?
        .into_iter()
        .find_map(|(name, at)| (name == key).then_some(at))
        .map_or(Value::Null, |at| json!(at));
    Ok(json!({ "value": value, "expiry": expiry }))
}

async fn settle(journal: &Shared, prefix: &str, expected: usize) -> Result<(), Box<HelperError>> {
    for _attempt in 0..SETTLE {
        let observed = lock(journal)
            .events
            .iter()
            .filter(|entry| entry.starts_with(prefix))
            .count();
        if observed >= expected {
            return Ok(());
        }
        yield_now().await;
    }
    Err(Box::<HelperError>::from(std::io::Error::other(
        "the subscriber never reached the expected events",
    )))
}

fn journal_value(journal: &Shared) -> Value {
    let mut guard = lock(journal);
    json!({
        "events": std::mem::take(&mut guard.events),
        "publications": std::mem::take(&mut guard.publications),
        "warnings": std::mem::take(&mut guard.warnings),
        "caches": Value::Object(std::mem::take(&mut guard.caches).into_iter().collect()),
    })
}
