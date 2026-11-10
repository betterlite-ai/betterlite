use std::collections::BTreeMap;
use std::future::ready;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use thiserror::Error;
use tracing::Subscriber;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::budget_primitives::window_start::DurationParser;
use crate::model_budget_values::budget_config::BudgetRuntime;
use crate::model_budget_values::candidates::BedrockCatalog;
use crate::model_budget_values::spend_reads::{CachedSpend, SpendCache};

pub(crate) struct FixedDurations;

impl DurationParser for FixedDurations {
    fn seconds(&self, spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        let units = [("s", 1), ("m", 60), ("h", 3_600), ("d", 86_400)];
        let (digits, factor) = units
            .iter()
            .find_map(|(suffix, factor)| Some((spec.strip_suffix(suffix)?, *factor)))?;
        digits.parse::<i64>().ok()?.checked_mul(factor)
    }
}

pub(crate) fn runtime() -> BudgetRuntime<'static> {
    BudgetRuntime {
        durations: &FixedDurations,
        now: DateTime::UNIX_EPOCH,
    }
}

pub(crate) struct FakeCatalog {
    pub(crate) bedrock_models: Vec<&'static str>,
}

impl BedrockCatalog for FakeCatalog {
    fn base_model(&self, model: &str) -> String {
        let routed = model.strip_prefix("bedrock/").unwrap_or(model);
        routed.strip_prefix("us.").unwrap_or(routed).to_owned()
    }

    fn prices_as_bedrock(&self, base_model: &str) -> bool {
        self.bedrock_models.contains(&base_model)
    }
}

pub(crate) fn catalog() -> FakeCatalog {
    FakeCatalog {
        bedrock_models: vec!["anthropic.claude-opus-4-8", "titan", "amazon."],
    }
}

pub(crate) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn budgets(text: &str) -> BTreeMap<String, JsonData> {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn amount(units: i64, scale: u32) -> Decimal {
    Decimal::new(units, scale)
}

struct WarnCounter(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for WarnCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::WARN {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) struct Warnings {
    count: Arc<AtomicUsize>,
    _guard: DefaultGuard,
}

impl Warnings {
    pub(crate) fn capture() -> Self {
        let count = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry().with(WarnCounter(Arc::clone(&count)));
        Self {
            count,
            _guard: set_default(subscriber),
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("fake cache failure")]
pub(crate) struct FakeFailure;

pub(crate) type FakeRedis = Option<Result<BTreeMap<String, CachedSpend>, FakeFailure>>;

pub(crate) struct FakeCache {
    redis: FakeRedis,
    memory: Option<Vec<Option<CachedSpend>>>,
    pub(crate) redis_reads: Mutex<Vec<Vec<String>>>,
    pub(crate) memory_reads: Mutex<Vec<Vec<String>>>,
}

impl FakeCache {
    pub(crate) fn new(redis: FakeRedis, memory: Option<Vec<Option<CachedSpend>>>) -> Self {
        Self {
            redis,
            memory,
            redis_reads: Mutex::new(Vec::new()),
            memory_reads: Mutex::new(Vec::new()),
        }
    }
}

impl SpendCache for FakeCache {
    type Error = FakeFailure;

    fn redis_batch_get(
        &self,
        keys: &[String],
    ) -> impl Future<Output = Result<Option<BTreeMap<String, CachedSpend>>, FakeFailure>> + Send
    {
        self.redis_reads.lock().unwrap().push(keys.to_vec());
        ready(self.redis.clone().transpose())
    }

    fn memory_batch_get(
        &self,
        keys: &[String],
    ) -> impl Future<Output = Option<Vec<Option<CachedSpend>>>> + Send {
        self.memory_reads.lock().unwrap().push(keys.to_vec());
        ready(self.memory.clone())
    }
}
