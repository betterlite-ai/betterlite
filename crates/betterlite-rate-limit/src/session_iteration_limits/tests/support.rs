#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
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
use thiserror::Error;

use crate::session_iteration_limits::KeyIterations;
use crate::session_iteration_limits::SessionIterationHandler;
use crate::session_iteration_limits::cache::SessionIterationCache;
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

#[derive(Debug, Error)]
#[error("the fake cache failed")]
pub(crate) struct FakeFailure;

pub(crate) struct FakeScript;

pub(crate) struct FakeCache {
    redis: bool,
    fails_script: AtomicBool,
    registered: Mutex<Vec<String>>,
    state: Mutex<FakeState>,
}

#[derive(Default)]
struct FakeState {
    clock: Duration,
    memory: BTreeMap<String, StoredCount>,
    memory_ttls: BTreeMap<String, Duration>,
    redis: BTreeMap<String, i64>,
    redis_ttls: BTreeMap<String, Duration>,
}

struct StoredCount {
    count: i64,
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
            fails_script: AtomicBool::new(false),
            registered: Mutex::new(Vec::new()),
            state: Mutex::new(FakeState::default()),
        }
    }

    pub(crate) fn fail_script(&self) {
        self.fails_script.store(true, Ordering::SeqCst);
    }

    pub(crate) fn preload_memory(&self, key: &str, count: i64) {
        let mut state = self.state.lock().unwrap();
        let expires_at = state.clock.saturating_add(Duration::from_secs(3600));
        state
            .memory
            .insert(key.to_owned(), StoredCount { count, expires_at });
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

    pub(crate) fn memory(&self, key: &str) -> Option<i64> {
        let state = self.state.lock().unwrap();
        let stored = state.memory.get(key)?;
        (state.clock < stored.expires_at).then_some(stored.count)
    }

    pub(crate) fn memory_ttl(&self, key: &str) -> Option<Duration> {
        self.state.lock().unwrap().memory_ttls.get(key).copied()
    }

    pub(crate) fn memory_keys(&self) -> Vec<String> {
        self.state.lock().unwrap().memory.keys().cloned().collect()
    }

    pub(crate) fn redis_count(&self, key: &str) -> Option<i64> {
        self.state.lock().unwrap().redis.get(key).copied()
    }

    pub(crate) fn redis_ttl(&self, key: &str) -> Option<Duration> {
        self.state.lock().unwrap().redis_ttls.get(key).copied()
    }

    pub(crate) fn registered(&self) -> Vec<String> {
        self.registered.lock().unwrap().clone()
    }
}

impl SessionIterationCache for FakeCache {
    type Error = FakeFailure;
    type Script = FakeScript;

    fn redis_configured(&self) -> bool {
        self.redis
    }

    fn register_increment_script(&self, script: &str) -> Option<FakeScript> {
        self.registered.lock().unwrap().push(script.to_owned());
        Some(FakeScript)
    }

    fn run_increment_script(
        &self,
        _script: &FakeScript,
        key: &str,
        ttl: Duration,
    ) -> impl Future<Output = Result<i64, FakeFailure>> + Send {
        if self.fails_script.load(Ordering::SeqCst) {
            return ready(Err(FakeFailure));
        }
        let mut state = self.state.lock().unwrap();
        let current = state.redis.get(key).copied().unwrap_or(0);
        let next = current.saturating_add(1);
        state.redis.insert(key.to_owned(), next);
        if next == 1 {
            state.redis_ttls.insert(key.to_owned(), ttl);
        }
        drop(state);
        ready(Ok(next))
    }

    fn memory_count(&self, key: &str) -> impl Future<Output = Option<i64>> + Send {
        ready(self.memory(key))
    }

    fn memory_set_count(
        &self,
        key: &str,
        value: i64,
        ttl: Duration,
    ) -> impl Future<Output = ()> + Send {
        let mut state = self.state.lock().unwrap();
        let expires_at = state.clock.saturating_add(ttl);
        state.memory.insert(
            key.to_owned(),
            StoredCount {
                count: value,
                expires_at,
            },
        );
        state.memory_ttls.insert(key.to_owned(), ttl);
        drop(state);
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

pub(crate) fn iteration_handler(cache: FakeCache) -> SessionIterationHandler<FakeCache> {
    iteration_handler_at(cache, Duration::from_secs(3600))
}

pub(crate) fn iteration_handler_at(
    cache: FakeCache,
    ttl: Duration,
) -> SessionIterationHandler<FakeCache> {
    SessionIterationHandler::new(cache, ttl, Box::new(FakeResolver))
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
