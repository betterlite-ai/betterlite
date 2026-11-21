#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::agent_registry::AgentRegistryApi as _;
use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_db::identity_rows::Payload;
use betterlite_error::rate_limit::ProxyRateLimitError;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ProviderIdentity;
use betterlite_models::resolution::ResolutionError;
use betterlite_models::resolution::ResolvedModel;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_counters::RedisCountersError;

use crate::limiter_records::{CacheCounterValue, ScriptReply};
use crate::rate_limit_window_store::{CounterCache, CounterScript, CounterValues, RedisCacheKind};
use crate::session_iteration_limits::INCREMENT_ITERATIONS_SCRIPT;
use crate::session_iteration_limits::KeyIterations;
use crate::session_iteration_limits::SessionIterationHandler;
use crate::session_iteration_limits::error::SessionIterationLimitError;

pub(crate) const AGENT_ID: &str = "agent-1";
pub(crate) const LIMIT_ONE: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_iterations":1}}"#;
pub(crate) const LIMIT_TWO: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_iterations":2}}"#;
pub(crate) const LIMIT_FIVE: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_iterations":5}}"#;
pub(crate) const LIMIT_TEXT: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_iterations":"2"}}"#;
pub(crate) const LIMIT_TRUE: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_iterations":true}}"#;
pub(crate) const LIMIT_FLOAT: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_iterations":2.7}}"#;
pub(crate) const LIMIT_NULL: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_iterations":null}}"#;
pub(crate) const LIMIT_INVALID: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_iterations":"soon"}}"#;
pub(crate) const LIMIT_LIST: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_iterations":[]}}"#;
pub(crate) const LIMIT_HUGE: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_iterations":9223372036854775808}}"#;
pub(crate) const LIMIT_ZERO: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_iterations":0}}"#;
pub(crate) const NO_LIMIT: &str = r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"other":true}}"#;
pub(crate) const NO_PARAMS: &str =
    r#"{"agent_id":"agent-1","agent_name":"alpha","agent_card_params":{"model":"gpt-4o"}}"#;

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(crate) enum ScriptMode {
    #[default]
    Counts,
    Fails,
    AnswersNil,
}

#[derive(Clone)]
pub(crate) struct FakeCache {
    redis: bool,
    state: Arc<Mutex<FakeState>>,
}

struct FakeScript {
    state: Arc<Mutex<FakeState>>,
}

#[derive(Default)]
struct FakeState {
    clock: Duration,
    script_mode: ScriptMode,
    memory: BTreeMap<String, StoredCount>,
    memory_ttls: BTreeMap<String, i64>,
    redis: BTreeMap<String, i64>,
    redis_ttls: BTreeMap<String, i64>,
}

struct StoredCount {
    value: CacheCounterValue,
    expires_at: Duration,
}

impl FakeCache {
    pub(crate) fn without_redis() -> Self {
        Self::new(false)
    }

    pub(crate) fn with_redis() -> Self {
        Self::new(true)
    }

    fn new(redis: bool) -> Self {
        Self {
            redis,
            state: Arc::new(Mutex::new(FakeState::default())),
        }
    }

    fn script(&self) -> Arc<dyn CounterScript> {
        Arc::new(FakeScript {
            state: Arc::clone(&self.state),
        })
    }

    pub(crate) fn set_script_mode(&self, mode: ScriptMode) {
        self.state.lock().unwrap().script_mode = mode;
    }

    pub(crate) fn preload_memory(&self, key: &str, count: i64) {
        self.preload_memory_value(key, CacheCounterValue::Int(count));
    }

    pub(crate) fn preload_memory_value(&self, key: &str, value: CacheCounterValue) {
        let mut state = self.state.lock().unwrap();
        let expires_at = state.clock.saturating_add(Duration::from_secs(3600));
        state
            .memory
            .insert(key.to_owned(), StoredCount { value, expires_at });
    }

    pub(crate) fn preload_redis(&self, key: &str, count: i64) {
        self.state
            .lock()
            .unwrap()
            .redis
            .insert(key.to_owned(), count);
    }

    pub(crate) fn advance(&self, by: Duration) {
        let mut state = self.state.lock().unwrap();
        state.clock = state.clock.saturating_add(by);
    }

    fn memory_value(&self, key: &str) -> Option<CacheCounterValue> {
        let state = self.state.lock().unwrap();
        let stored = state.memory.get(key)?;
        (state.clock < stored.expires_at).then(|| stored.value.clone())
    }

    pub(crate) fn memory(&self, key: &str) -> Option<i64> {
        match self.memory_value(key)? {
            CacheCounterValue::Int(count) => Some(count),
            CacheCounterValue::Float(_)
            | CacheCounterValue::Text(_)
            | CacheCounterValue::Bytes(_) => panic!("the handler writes whole counts"),
        }
    }

    pub(crate) fn memory_ttl(&self, key: &str) -> Option<i64> {
        self.state.lock().unwrap().memory_ttls.get(key).copied()
    }

    pub(crate) fn memory_keys(&self) -> Vec<String> {
        self.state.lock().unwrap().memory.keys().cloned().collect()
    }

    pub(crate) fn redis_count(&self, key: &str) -> Option<i64> {
        self.state.lock().unwrap().redis.get(key).copied()
    }

    pub(crate) fn redis_ttl(&self, key: &str) -> Option<i64> {
        self.state.lock().unwrap().redis_ttls.get(key).copied()
    }
}

#[async_trait::async_trait]
impl CounterCache for FakeCache {
    async fn get_cache(&self, key: &str, local_only: bool) -> Option<CacheCounterValue> {
        assert!(local_only, "the hook reads only the local counter");
        self.memory_value(key)
    }

    async fn set_cache(&self, key: &str, value: CacheCounterValue, ttl: i64, local_only: bool) {
        assert!(local_only, "the hook writes only the local counter");
        let lifetime = Duration::from_secs(u64::try_from(ttl).unwrap());
        let mut state = self.state.lock().unwrap();
        let expires_at = state.clock.saturating_add(lifetime);
        state
            .memory
            .insert(key.to_owned(), StoredCount { value, expires_at });
        state.memory_ttls.insert(key.to_owned(), ttl);
    }

    async fn batch_get_cache(&self, _keys: &[String], _local_only: bool) -> Option<CounterValues> {
        panic!("the hook reads one counter at a time")
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        self.redis.then_some(RedisCacheKind::Standalone)
    }
}

#[async_trait::async_trait]
impl CounterScript for FakeScript {
    fn source(&self) -> &'static str {
        INCREMENT_ITERATIONS_SCRIPT
    }

    async fn run(
        &self,
        keys: &[String],
        args: &[RedisScalar],
    ) -> Result<ScriptReply, RedisCountersError> {
        let mut state = self.state.lock().unwrap();
        match state.script_mode {
            ScriptMode::Counts => {}
            ScriptMode::Fails => {
                return Err(RedisCountersError::UnexpectedReply {
                    operation: "the fake increment script",
                });
            }
            ScriptMode::AnswersNil => return Ok(ScriptReply::Nil),
        }
        let [key] = keys else {
            panic!("the script takes one key");
        };
        let [RedisScalar::Text(ttl)] = args else {
            panic!("the script takes the ttl as its one argument");
        };
        let current = state.redis.get(key).copied().unwrap_or(0);
        let next = current.saturating_add(1);
        state.redis.insert(key.clone(), next);
        if next == 1 {
            state.redis_ttls.insert(key.clone(), ttl.parse().unwrap());
        }
        drop(state);
        Ok(ScriptReply::Int(next))
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

pub(crate) fn iteration_handler(cache: FakeCache) -> SessionIterationHandler {
    iteration_handler_at(cache, Duration::from_secs(3600))
}

pub(crate) fn iteration_handler_at(cache: FakeCache, ttl: Duration) -> SessionIterationHandler {
    let script = cache.script();
    SessionIterationHandler::new(Arc::new(cache), Some(script), ttl, Box::new(FakeResolver))
}

pub(crate) fn agent_registry_with(records: &[&str]) -> AgentRegistry {
    let registry = AgentRegistry::default();
    for record in records {
        let agent: AgentResponse = serde_json::from_str(record).expect("the agent record is valid");
        registry.register_agent(agent);
    }
    registry
}

pub(crate) fn agent_key() -> KeyIterations<'static> {
    KeyIterations {
        agent_id: Some(AGENT_ID),
        metadata: None,
    }
}

pub(crate) fn key_metadata(text: &str) -> Payload {
    serde_json::from_str(text).expect("the fixture is key metadata")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is json")
}

pub(crate) fn session_data(session: &str) -> Json {
    json(&format!(
        r#"{{"model":"gpt-4o","metadata":{{"session_id":"{session}"}}}}"#
    ))
}

pub(crate) fn memory_key(session: &str) -> String {
    format!("{{session_iterations:{session}}}:count")
}

pub(crate) fn expected_detail(session: &str, current_count: i64, max_iterations: i64) -> String {
    format!(
        "Max iterations exceeded for session {session}. Current count: {current_count}, max_iterations: {max_iterations}."
    )
}

pub(crate) fn rejection(error: SessionIterationLimitError) -> Box<ProxyRateLimitError> {
    let SessionIterationLimitError::Exceeded(limit) = error else {
        panic!("expected a session iteration rejection");
    };
    limit
}
