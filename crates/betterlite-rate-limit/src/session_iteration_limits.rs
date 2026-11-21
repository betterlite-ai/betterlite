mod admission;
pub mod error;
mod fields;
mod increment;
mod tests;

use std::sync::Arc;
use std::time::Duration;

use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::rejection_provider::DeploymentSnapshot;
use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_db::identity_rows::Payload as KeyMetadata;
use betterlite_models::decisions::Json;

use crate::rate_limit_window_store::{CounterCache, CounterScript};

use error::SessionIterationLimitError;

// LiteLLM's MAX_ITERATIONS_INCREMENT_SCRIPT: EXPIRE runs only when the counter did not exist before.
pub const INCREMENT_ITERATIONS_SCRIPT: &str = "local key = KEYS[1] local ttl = tonumber(ARGV[1]) local current = redis.call('INCR', key) if current == 1 then redis.call('EXPIRE', key, ttl) end return current";

// the agent litellm_params entry and the key metadata entry this hook enforces
pub(crate) const MAX_ITERATIONS: &str = "max_iterations";

// UserAPIKeyAuth's agent_id and metadata, the two fields _get_max_iterations reads
#[derive(Clone, Copy, Debug)]
pub struct KeyIterations<'auth> {
    pub agent_id: Option<&'auth str>,
    pub metadata: Option<&'auth KeyMetadata>,
}

pub struct SessionIterationHandler {
    pub cache: Arc<dyn CounterCache>,
    pub increment_script: Option<Arc<dyn CounterScript>>,
    pub ttl: Duration,
    pub resolver: Box<dyn ProviderResolution>,
}

impl SessionIterationHandler {
    // __init__: the script registered from INCREMENT_ITERATIONS_SCRIPT is kept only while the cache has Redis.
    #[must_use]
    pub fn new(
        cache: Arc<dyn CounterCache>,
        registered_script: Option<Arc<dyn CounterScript>>,
        ttl: Duration,
        resolver: Box<dyn ProviderResolution>,
    ) -> Self {
        let increment_script = registered_script.filter(|_| cache.redis_cache_kind().is_some());
        Self {
            cache,
            increment_script,
            ttl,
            resolver,
        }
    }

    // async_pre_call_hook: the session is counted before the call, and a count over its limit is the only rejection.
    pub async fn async_pre_call_hook(
        &self,
        registry: &AgentRegistry,
        router: Option<&dyn DeploymentSnapshot>,
        key: KeyIterations<'_>,
        data: &Json,
    ) -> Result<(), SessionIterationLimitError> {
        let Some(session_id) = fields::session_id_from(data) else {
            return Ok(());
        };
        let Some(max_iterations) = admission::max_iterations_for(registry, key)? else {
            return Ok(());
        };
        let current_count = increment::count_after_increment(self, &session_id).await?;
        if current_count <= max_iterations {
            return Ok(());
        }
        let request = admission::ExceededRequest {
            session_id: session_id.as_str(),
            current_count,
            max_iterations,
            data,
        };
        Err(admission::exceeded_error(self, router, &request))
    }
}
