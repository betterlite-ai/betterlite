mod admission;
pub mod cache;
pub mod error;
mod fields;
mod increment;
mod tests;

use std::time::Duration;

use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::rejection_provider::DeploymentSnapshot;
use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_db::identity_rows::Payload as KeyMetadata;
use betterlite_models::decisions::Json;

use cache::SessionIterationCache;
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

pub struct SessionIterationHandler<C: SessionIterationCache> {
    pub cache: C,
    pub script: Option<C::Script>,
    pub ttl: Duration,
    pub resolver: Box<dyn ProviderResolution>,
}

impl<C: SessionIterationCache> SessionIterationHandler<C> {
    // __init__: the Lua script is registered once, and only a configured Redis cache can hold it.
    #[must_use]
    pub fn new(cache: C, ttl: Duration, resolver: Box<dyn ProviderResolution>) -> Self {
        let script = cache
            .redis_configured()
            .then(|| cache.register_increment_script(INCREMENT_ITERATIONS_SCRIPT))
            .flatten();
        Self {
            cache,
            script,
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
        let current_count = increment::count_after_increment(self, session_id).await;
        if current_count <= max_iterations {
            return Ok(());
        }
        let request = admission::ExceededRequest {
            session_id,
            current_count,
            max_iterations,
            data,
        };
        Err(admission::exceeded_error(self, router, &request))
    }
}
