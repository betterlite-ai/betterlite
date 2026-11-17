mod admission;
pub mod cache;
mod charging;
pub mod error;
mod fields;
pub mod settings;
mod spend;
mod tests;

use std::time::Duration;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use crate::agent_registry::AgentRegistry;
use crate::rejection_provider::DeploymentSnapshot;
use crate::rejection_provider::ProviderResolution;

use cache::SessionBudgetCache;
use error::SessionBudgetError;

// LiteLLM's MAX_BUDGET_SESSION_INCREMENT_SCRIPT: EXPIRE runs only when the key did not exist before.
pub const INCREMENT_SPEND_SCRIPT: &str = "local key = KEYS[1] local amount = ARGV[1] local ttl = tonumber(ARGV[2]) local existed = redis.call('EXISTS', key) local new_val = redis.call('INCRBYFLOAT', key, amount) if existed == 0 then redis.call('EXPIRE', key, ttl) end return new_val";

// the agent litellm_params key this hook enforces, in dollars per session
pub(crate) const MAX_BUDGET_PER_SESSION: &str = "max_budget_per_session";

// kwargs["litellm_params"] plus kwargs["response_cost"], the fields async_log_success_event reads
pub struct SuccessEvent<'input> {
    pub litellm_params: Option<&'input Json>,
    pub response_cost: Option<Decimal>,
}

pub struct SessionBudgetHandler<C: SessionBudgetCache> {
    pub cache: C,
    pub script: Option<C::Script>,
    pub ttl: Duration,
    pub resolver: Box<dyn ProviderResolution>,
}

impl<C: SessionBudgetCache> SessionBudgetHandler<C> {
    // __init__: the Lua script is registered once, and only a configured Redis cache can hold it.
    #[must_use]
    pub fn new(cache: C, ttl: Duration, resolver: Box<dyn ProviderResolution>) -> Self {
        let script = if cache.redis_configured() {
            cache.register_increment_script(INCREMENT_SPEND_SCRIPT)
        } else {
            None
        };
        Self {
            cache,
            script,
            ttl,
            resolver,
        }
    }

    // async_pre_call_hook: the session's spend is checked before the call, and a 429 is the only rejection.
    pub async fn async_pre_call_hook(
        &self,
        registry: &AgentRegistry,
        router: Option<&dyn DeploymentSnapshot>,
        agent_id: Option<&str>,
        data: &Json,
    ) -> Result<(), SessionBudgetError> {
        let max_budget = admission::max_budget_for(registry, agent_id)?;
        let Some(session_id) = admission::session_id_from(data) else {
            return Ok(());
        };
        let Some(budget) = max_budget else {
            return Ok(());
        };
        let cache_key = format!("{{session_budget:{session_id}}}:spend");
        let current_spend = spend::current_spend(self, &cache_key).await;
        if current_spend < budget {
            return Ok(());
        }
        let request = admission::ExceededRequest {
            session_id,
            current_spend,
            max_budget: budget,
            data,
        };
        Err(admission::exceeded_error(self, router, &request))
    }

    // async_log_success_event: a success charges the session, and a cache failure only logs a warning.
    pub async fn async_log_success_event(
        &self,
        registry: &AgentRegistry,
        event: &SuccessEvent<'_>,
    ) {
        let Some(charge) = charging::charge_for(registry, event) else {
            return;
        };
        let cache_key = format!("{{session_budget:{}}}:spend", charge.session_id);
        spend::add_spend(self, &cache_key, charge.amount).await;
    }
}
