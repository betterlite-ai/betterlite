mod fakes;
mod subscriber;
mod warnings;

use std::collections::BTreeMap;
use std::mem::take;
use std::sync::{Arc, Mutex, MutexGuard};

use betterlite_auth::auth_pubsub::message::Invalidation;
use betterlite_auth::auth_pubsub::publish::{
    MAX_PENDING_PUBLISHES, PublishTasks, evict_and_broadcast, publish_invalidation,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use self::fakes::{Endpoint, Eviction, PublishMode};
use self::warnings::Warnings;

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

#[derive(Deserialize, Clone)]
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
        let outcome = with_default(subscriber, || {
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
        subscriber::subscriber_section(call, &journal).await?;
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

fn journal_value(journal: &Shared) -> Value {
    let mut guard = lock(journal);
    json!({
        "events": take(&mut guard.events),
        "publications": take(&mut guard.publications),
        "warnings": take(&mut guard.warnings),
        "caches": Value::Object(take(&mut guard.caches).into_iter().collect()),
    })
}
