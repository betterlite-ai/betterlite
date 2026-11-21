mod fake_cache;
mod input;
mod provider_table;

use std::fmt::Display;

use axum::response::IntoResponse as _;
use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::agent_registry::AgentRegistryApi as _;
use betterlite_config::sections::session_iteration_limits::max_iterations_ttl;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_rate_limit::session_iteration_limits::KeyIterations;
use betterlite_rate_limit::session_iteration_limits::SessionIterationHandler;
use betterlite_rate_limit::session_iteration_limits::error::SessionIterationLimitError;
use serde_json::Value;
use serde_json::json;
use tokio::runtime::Builder;

use fake_cache::FakeCache;
use input::Call;
use input::Input;
use provider_table::ProviderTable;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(failure)?;
        runtime.block_on(execute(input))
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

async fn execute(input: &Input) -> HelperResult {
    let ttl = max_iterations_ttl(input.ttl_env.as_deref()).map_err(failure)?;
    let cache = FakeCache::new(input.redis, &input.memory, &input.redis_values);
    let handler =
        SessionIterationHandler::new(cache, ttl, Box::new(ProviderTable::litellm_defaults()));
    let registry = AgentRegistry::default();
    for agent in &input.agents {
        registry.register_agent(agent.clone());
    }
    let mut outcomes: Vec<Value> = Vec::new();
    for call in &input.calls {
        outcomes.push(pre_call_outcome(&handler, &registry, call).await);
    }
    let snapshot = handler.cache.snapshot();
    Ok(json!({
        "ttl": handler.ttl.as_secs(),
        "script_registered": handler.script.is_some(),
        "calls": outcomes,
        "memory": snapshot.memory,
        "memory_ttls": snapshot.memory_ttls,
        "redis": snapshot.redis,
        "redis_ttls": snapshot.redis_ttls,
    }))
}

async fn pre_call_outcome(
    handler: &SessionIterationHandler<FakeCache>,
    registry: &AgentRegistry,
    call: &Call,
) -> Value {
    let key = KeyIterations {
        agent_id: call.agent_id.as_deref(),
        metadata: call.key_metadata.as_ref(),
    };
    match handler
        .async_pre_call_hook(registry, None, key, &call.data)
        .await
    {
        Ok(()) => json!({"kind": "pre_call", "outcome": "admitted"}),
        Err(SessionIterationLimitError::Exceeded(error)) => {
            let status_code = error.clone().into_response().status().as_u16();
            json!({
                "kind": "pre_call",
                "outcome": "exceeded",
                "status_code": status_code,
                "category": error.category,
                "rate_limit_type": error.rate_limit_type,
                "model": error.model,
                "llm_provider": error.llm_provider,
                "detail": error.detail,
            })
        }
        Err(other) => json!({"kind": "pre_call", "outcome": "error", "message": other.to_string()}),
    }
}
