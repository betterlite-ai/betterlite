use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use betterlite_auth::auth_cache::gateway_cache;
use betterlite_auth::auth_pubsub::subscriber::{CacheInvalidationSubscriber, Session};
use betterlite_cache::backend::BackendError;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use serde_json::{Value, json};
use tokio::task::yield_now;

use super::fakes::{Clock, Endpoint, PublishMode, Step};
use super::{Shared, StepInput, SubscriberCall, lock};

const CLOCK_NOW: f64 = 1000.0_f64;
const SETTLE: usize = 1000;
const SETTINGS: DualCacheSettings = DualCacheSettings {
    default_in_memory_ttl: None,
    default_redis_ttl: None,
    batch_cache_expiry: None,
    max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
    redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
};

struct Memories {
    ordinary: Arc<LocalMemoryOps>,
    key_objects: Arc<LocalMemoryOps>,
    additional: Arc<LocalMemoryOps>,
}

fn seeded_memories(keys: &[String]) -> Result<Memories, Box<HelperError>> {
    let now: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| CLOCK_NOW);
    let ordinary = Arc::new(LocalMemoryOps::new(1024, Arc::clone(&now)).or_value_error()?);
    let key_objects = Arc::new(LocalMemoryOps::new(1024, Arc::clone(&now)).or_value_error()?);
    let additional = Arc::new(LocalMemoryOps::new(1024, now).or_value_error()?);
    seed(&ordinary, keys, 2.0_f64)?;
    seed(&key_objects, keys, 1.0_f64)?;
    seed(&additional, keys, 3.0_f64)?;
    Ok(Memories {
        ordinary,
        key_objects,
        additional,
    })
}

fn scripted_subscriber(
    journal: &Shared,
    sessions: Vec<Vec<Step>>,
    memories: &Memories,
) -> Arc<CacheInvalidationSubscriber<Endpoint, Clock, DualCache<LocalMemoryOps>>> {
    Arc::new(CacheInvalidationSubscriber {
        endpoint: Arc::new(Endpoint::new(
            Arc::clone(journal),
            sessions,
            PublishMode::Accept,
        )),
        clock: Clock,
        user_cache: Arc::new(gateway_cache(
            Arc::clone(&memories.ordinary),
            Arc::clone(&memories.key_objects),
            None,
            SETTINGS,
            45.0_f64,
        )),
        additional: vec![Arc::clone(&memories.additional)],
        session: Mutex::new(Session::default()),
    })
}

pub(super) async fn subscriber_section(
    call: &SubscriberCall,
    journal: &Shared,
) -> Result<(), Box<HelperError>> {
    let probe_keys: &[String] = call.probe_keys.as_deref().unwrap_or_default();
    let memories = seeded_memories(probe_keys)?;
    let sessions = call
        .sessions
        .iter()
        .map(|steps| session_plan(steps))
        .collect();
    let subscriber = scripted_subscriber(journal, sessions, &memories);
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
            ("ordinary", memories.ordinary.as_ref()),
            ("key_objects", memories.key_objects.as_ref()),
            ("additional:0", memories.additional.as_ref()),
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
    Err(Box::<HelperError>::from(io::Error::other(
        "the subscriber never reached the expected events",
    )))
}
