use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_error::rate_limit::ProxyRateLimitError;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ProviderIdentity;
use betterlite_models::resolution::ResolutionError;
use betterlite_models::resolution::ResolvedModel;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::agent_policy::AgentResponse;
use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;
use crate::model_budget_values::spend_reads::CachedSpend;
use crate::rejection_provider::ProviderResolution;
use crate::session_budget::SessionBudgetHandler;
use crate::session_budget::SuccessEvent;
use crate::session_budget::cache::SessionBudgetCache;
use crate::session_budget::error::SessionBudgetError;

pub(crate) const BUDGET_ONE: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_budget_per_session":1.0}}"#;
pub(crate) const BUDGET_ZERO: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_budget_per_session":0}}"#;
pub(crate) const BUDGET_TEXT: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_budget_per_session":"0.5"}}"#;
pub(crate) const BUDGET_TRUE: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_budget_per_session":true}}"#;
pub(crate) const BUDGET_NULL: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_budget_per_session":null}}"#;
pub(crate) const BUDGET_INVALID: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_budget_per_session":"soon"}}"#;
pub(crate) const BUDGET_INFINITY: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_budget_per_session":"inf"}}"#;
pub(crate) const BUDGET_FALSE: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_budget_per_session":false}}"#;
pub(crate) const BUDGET_LIST: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_budget_per_session":[]}}"#;
pub(crate) const BUDGET_ROUNDING: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_budget_per_session":2.675}}"#;
pub(crate) const NO_BUDGET: &str = r#"{"agent_id":"agent-2","agent_name":"beta","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"other":true}}"#;

#[derive(Debug, Error)]
#[error("the fake cache failed")]
pub(crate) struct FakeFailure;

pub(crate) struct FakeScript;

#[derive(Clone)]
pub(crate) struct ScriptRun {
    pub(crate) key: String,
    pub(crate) amount: Decimal,
    pub(crate) ttl: Duration,
}

pub(crate) struct FakeCache {
    pub(crate) redis: bool,
    pub(crate) holds_script: AtomicBool,
    pub(crate) fails_redis_read: AtomicBool,
    pub(crate) fails_script: AtomicBool,
    pub(crate) registered: Mutex<Vec<String>>,
    pub(crate) memory: Mutex<BTreeMap<String, CachedSpend>>,
    pub(crate) redis_values: Mutex<BTreeMap<String, CachedSpend>>,
    pub(crate) memory_ttls: Mutex<BTreeMap<String, Duration>>,
    pub(crate) script_runs: Mutex<Vec<ScriptRun>>,
}

impl FakeCache {
    pub(crate) fn new() -> Self {
        Self {
            redis: false,
            holds_script: AtomicBool::new(true),
            fails_redis_read: AtomicBool::new(false),
            fails_script: AtomicBool::new(false),
            registered: Mutex::new(Vec::new()),
            memory: Mutex::new(BTreeMap::new()),
            redis_values: Mutex::new(BTreeMap::new()),
            memory_ttls: Mutex::new(BTreeMap::new()),
            script_runs: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn with_redis() -> Self {
        let mut cache = Self::new();
        cache.redis = true;
        cache
    }

    pub(crate) fn preload_memory(&self, key: &str, amount: Decimal) {
        self.memory
            .lock()
            .unwrap()
            .insert(key.to_owned(), CachedSpend::Number(amount));
    }

    pub(crate) fn preload_redis(&self, key: &str, amount: Decimal) {
        self.redis_values
            .lock()
            .unwrap()
            .insert(key.to_owned(), CachedSpend::Number(amount));
    }

    pub(crate) fn preload_redis_text(&self, key: &str, text: &str) {
        self.redis_values
            .lock()
            .unwrap()
            .insert(key.to_owned(), CachedSpend::Text(text.to_owned()));
    }

    pub(crate) fn preload_memory_text(&self, key: &str, text: &str) {
        self.memory
            .lock()
            .unwrap()
            .insert(key.to_owned(), CachedSpend::Text(text.to_owned()));
    }

    pub(crate) fn preload_memory_unreadable(&self, key: &str) {
        self.memory
            .lock()
            .unwrap()
            .insert(key.to_owned(), CachedSpend::Unreadable);
    }

    pub(crate) fn preload_redis_unreadable(&self, key: &str) {
        self.redis_values
            .lock()
            .unwrap()
            .insert(key.to_owned(), CachedSpend::Unreadable);
    }

    pub(crate) fn memory(&self, key: &str) -> Option<Decimal> {
        spend_of(self.memory.lock().unwrap().get(key)?)
    }

    pub(crate) fn redis(&self, key: &str) -> Option<Decimal> {
        spend_of(self.redis_values.lock().unwrap().get(key)?)
    }

    pub(crate) fn memory_ttl(&self, key: &str) -> Option<Duration> {
        self.memory_ttls.lock().unwrap().get(key).copied()
    }
}

fn spend_of(value: &CachedSpend) -> Option<Decimal> {
    match value {
        CachedSpend::Number(amount) => Some(*amount),
        CachedSpend::Text(text) => text.parse().ok(),
        CachedSpend::Unreadable => None,
    }
}

impl SessionBudgetCache for FakeCache {
    type Error = FakeFailure;
    type Script = FakeScript;

    fn redis_configured(&self) -> bool {
        self.redis
    }

    fn register_increment_script(&self, script: &str) -> Option<FakeScript> {
        if !self.holds_script.load(Ordering::SeqCst) {
            return None;
        }
        self.registered.lock().unwrap().push(script.to_owned());
        Some(FakeScript)
    }

    fn redis_spend(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<CachedSpend>, FakeFailure>> + Send {
        if self.fails_redis_read.load(Ordering::SeqCst) {
            return ready(Err(FakeFailure));
        }
        ready(Ok(self.redis_values.lock().unwrap().get(key).cloned()))
    }

    fn run_increment_script(
        &self,
        _script: &FakeScript,
        key: &str,
        amount: Decimal,
        _ttl: Duration,
    ) -> impl Future<Output = Result<Decimal, FakeFailure>> + Send {
        if self.fails_script.load(Ordering::SeqCst) {
            return ready(Err(FakeFailure));
        }
        self.script_runs.lock().unwrap().push(ScriptRun {
            key: key.to_owned(),
            amount,
            ttl,
        });
        let mut values = self.redis_values.lock().unwrap();
        let current = values.get(key).and_then(spend_of).unwrap_or(Decimal::ZERO);
        let next = current.checked_add(amount).unwrap_or(Decimal::MAX);
        values.insert(key.to_owned(), CachedSpend::Number(next));
        drop(values);
        ready(Ok(next))
    }

    fn memory_spend(&self, key: &str) -> impl Future<Output = Option<CachedSpend>> + Send {
        ready(self.memory.lock().unwrap().get(key).cloned())
    }

    fn memory_set_spend(
        &self,
        key: &str,
        value: CachedSpend,
        ttl: Duration,
    ) -> impl Future<Output = ()> + Send {
        self.memory.lock().unwrap().insert(key.to_owned(), value);
        self.memory_ttls.lock().unwrap().insert(key.to_owned(), ttl);
        ready(())
    }
}

pub(crate) struct FakeResolver;

impl ProviderResolution for FakeResolver {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        if model != "gpt-4o" {
            return Err(ResolutionError::MissingProvider {
                model: model.to_owned(),
            });
        }
        Ok(ResolvedModel {
            model: model.to_owned(),
            provider: ProviderIdentity::Custom("openai".to_owned()),
            api_base: None,
            api_key: None,
        })
    }
}

pub(crate) fn handler(cache: FakeCache) -> SessionBudgetHandler<FakeCache> {
    handler_at(cache, Duration::from_secs(3600))
}

pub(crate) fn handler_at(cache: FakeCache, ttl: Duration) -> SessionBudgetHandler<FakeCache> {
    SessionBudgetHandler::new(cache, ttl, Box::new(FakeResolver))
}

pub(crate) fn agent(record: &str) -> AgentResponse {
    serde_json::from_str(record).expect("the agent record is valid")
}

pub(crate) fn agent_registry_with(records: &[&str]) -> AgentRegistry {
    let registry = AgentRegistry::default();
    for record in records {
        registry.register_agent(agent(record));
    }
    registry
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is json")
}

pub(crate) fn amount(text: &str) -> Decimal {
    text.parse().expect("the fixture is a decimal")
}

pub(crate) fn success(
    litellm_params: Option<&Json>,
    response_cost: Option<Decimal>,
) -> SuccessEvent<'_> {
    SuccessEvent {
        litellm_params,
        response_cost,
    }
}

pub(crate) fn rejection(error: SessionBudgetError) -> Box<ProxyRateLimitError> {
    let SessionBudgetError::Exceeded(limit) = error else {
        panic!("expected a session budget rejection");
    };
    limit
}
