mod fake_cache;
pub(crate) mod input;

use std::fmt::Display;

use axum::response::IntoResponse as _;
use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::agent_registry::AgentRegistryApi as _;
use betterlite_auth::model_budget_values::amount_text::amount_from_text;
use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_auth::session_budget::SessionBudgetHandler;
use betterlite_auth::session_budget::SuccessEvent;
use betterlite_auth::session_budget::error::SessionBudgetError;
use betterlite_auth::session_budget::settings::session_budget_ttl;
use betterlite_models::resolution::ProviderIdentity;
use betterlite_models::resolution::ResolutionError;
use betterlite_models::resolution::ResolvedModel;
use serde_json::Value;
use serde_json::json;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use fake_cache::FakeCache;
use input::Charge;
use input::Input;
use input::PreCall;

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

// the two model spellings both sides agree on: litellm.get_llm_provider resolves gpt-4o to openai
struct Resolver;

impl ProviderResolution for Resolver {
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

async fn execute(input: &Input) -> HelperResult {
    let ttl = session_budget_ttl(input.ttl_env.as_deref()).map_err(failure)?;
    let cache = FakeCache::new(input.redis, &input.memory, &input.redis_values)?;
    let handler = SessionBudgetHandler::new(cache, ttl, Box::new(Resolver));
    let registry = AgentRegistry::default();
    for record in &input.agents {
        registry.register_agent(record.clone());
    }
    let mut outcomes: Vec<Value> = Vec::new();
    for call in &input.pre_calls {
        outcomes.push(pre_call_outcome(&handler, &registry, call).await);
    }
    for call in &input.charges {
        outcomes.push(charge_outcome(&handler, &registry, call).await);
    }
    let (memory, memory_ttls) = handler.cache.memory_state();
    let (redis, redis_ttls) = handler.cache.redis_state();
    Ok(json!({
        "ttl": handler.ttl.as_secs(),
        "script_registered": handler.script.is_some(),
        "calls": outcomes,
        "memory": memory,
        "memory_ttls": memory_ttls,
        "redis": redis,
        "redis_ttls": redis_ttls,
    }))
}

async fn pre_call_outcome(
    handler: &SessionBudgetHandler<FakeCache>,
    registry: &AgentRegistry,
    call: &PreCall,
) -> Value {
    let outcome = handler
        .async_pre_call_hook(registry, None, call.agent_id.as_deref(), &call.data)
        .await;
    match outcome {
        Ok(()) => json!({"kind": "pre_call", "outcome": "admitted"}),
        Err(SessionBudgetError::Exceeded(error)) => {
            let rejection = *error;
            let status_code = rejection.clone().into_response().status().as_u16();
            json!({
                "kind": "pre_call",
                "outcome": "exceeded",
                "status_code": status_code,
                "category": rejection.category,
                "rate_limit_type": rejection.rate_limit_type,
                "model": rejection.model,
                "llm_provider": rejection.llm_provider,
            })
        }
        Err(other) => json!({"kind": "pre_call", "outcome": "error", "message": other.to_string()}),
    }
}

async fn charge_outcome(
    handler: &SessionBudgetHandler<FakeCache>,
    registry: &AgentRegistry,
    call: &Charge,
) -> Value {
    let cost = call
        .response_cost
        .as_ref()
        .and_then(|number| amount_from_text(&number.to_string()));
    let event = SuccessEvent {
        litellm_params: call.litellm_params.as_ref(),
        response_cost: cost,
    };
    handler.async_log_success_event(registry, &event).await;
    json!({"kind": "charge", "outcome": "logged"})
}
