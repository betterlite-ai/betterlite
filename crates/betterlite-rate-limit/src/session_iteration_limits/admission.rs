use betterlite_auth::agent::get_agent_by_id;
use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::budget_counter::coercion::IntInput;
use betterlite_auth::provider_resolution::resolve_rate_limit_provider;
use betterlite_auth::rejection_provider::DeploymentSnapshot;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload as KeyMetadata;
use betterlite_error::rate_limit::ProxyRateLimitErrorInit;
use betterlite_error::rate_limit::RateLimitDimension;
use betterlite_models::decisions::Json;
use betterlite_redact::structured::Payload;
use serde_json::Number;

use super::KeyIterations;
use super::MAX_ITERATIONS;
use super::SessionIterationHandler;
use super::error::SessionIterationLimitError;

// the fields the 429 body and its model/provider pair are built from
pub(crate) struct ExceededRequest<'input> {
    pub(crate) session_id: &'input str,
    pub(crate) current_count: i64,
    pub(crate) max_iterations: i64,
    pub(crate) data: &'input Json,
}

// _get_max_iterations: the agent's litellm_params entry first, then the key's metadata, and none when neither sets one
pub(crate) fn max_iterations_for(
    registry: &AgentRegistry,
    key: KeyIterations<'_>,
) -> Result<Option<i64>, SessionIterationLimitError> {
    if let Some(agent_id) = key.agent_id {
        let agent_value = agent_iterations(registry, agent_id)?;
        if let Some(value) = agent_value {
            return iterations_of(&value, agent_id);
        }
    }
    key_iterations(key.metadata)
}

// the agent's configured entry, and none when the agent, the entry or a null value leaves the limit unset
fn agent_iterations(
    registry: &AgentRegistry,
    agent_id: &str,
) -> Result<Option<Json>, SessionIterationLimitError> {
    let Some(agent) = get_agent_by_id(registry, agent_id) else {
        return Ok(None);
    };
    let Some(raw) = agent.litellm_params.get(MAX_ITERATIONS) else {
        return Ok(None);
    };
    let value: Json = serde_json::from_str(raw.get()).map_err(|source| {
        SessionIterationLimitError::IterationsJson {
            agent_id: agent_id.to_owned(),
            source,
        }
    })?;
    if value == Json::Null {
        return Ok(None);
    }
    Ok(Some(value))
}

// int() on the agent's value: a number, a boolean or integer text counts, and anything else is an error
fn iterations_of(value: &Json, agent_id: &str) -> Result<Option<i64>, SessionIterationLimitError> {
    let count = match value {
        Json::Null => return Ok(None),
        Json::Bool(flag) => Some(i64::from(*flag)),
        Json::Number(number) => count_of(number),
        Json::Text(text) => text.as_str().int_value(),
        Json::List(_) | Json::Object(_) => None,
    };
    count
        .map(Some)
        .ok_or_else(|| SessionIterationLimitError::InvalidAgentIterations {
            agent_id: agent_id.to_owned(),
        })
}

// int() on the key's metadata entry, the backwards-compatible limit _get_max_iterations falls back to
fn key_iterations(
    metadata: Option<&KeyMetadata>,
) -> Result<Option<i64>, SessionIterationLimitError> {
    let Some(entry) = metadata.and_then(|payload| payload.0.get(MAX_ITERATIONS)) else {
        return Ok(None);
    };
    match entry {
        JsonData::Null => Ok(None),
        JsonData::Bool(flag) => Ok(Some(i64::from(*flag))),
        JsonData::Number(number) => count_of(number)
            .map(Some)
            .ok_or(SessionIterationLimitError::InvalidKeyIterations),
        JsonData::String(text) => text
            .as_str()
            .int_value()
            .map(Some)
            .ok_or(SessionIterationLimitError::InvalidKeyIterations),
        JsonData::Array(_) | JsonData::Object(_) => {
            Err(SessionIterationLimitError::InvalidKeyIterations)
        }
    }
}

// int() on a JSON number: an integer is taken as it is, and a float truncates toward zero
fn count_of(number: &Number) -> Option<i64> {
    if let Some(integer) = number.as_i64() {
        return Some(integer);
    }
    IntInput::int_value(number.as_f64()?)
}

// data.get("model"): the model the rejected request named, when it is a string
fn requested_model(data: &Json) -> Option<&str> {
    let Json::Object(fields) = data else {
        return None;
    };
    let Json::Text(model) = fields.get("model")? else {
        return None;
    };
    Some(model.as_str())
}

// the 429 async_pre_call_hook raises: RateLimitType.MAX_ITERATIONS with the resolved model and provider
pub(crate) fn exceeded_error(
    handler: &SessionIterationHandler,
    router: Option<&dyn DeploymentSnapshot>,
    request: &ExceededRequest<'_>,
) -> SessionIterationLimitError {
    let (model, llm_provider) = resolve_rate_limit_provider(
        requested_model(request.data),
        handler.resolver.as_ref(),
        router,
    );
    SessionIterationLimitError::rejected(ProxyRateLimitErrorInit {
        detail: Payload::String(format!(
            "Max iterations exceeded for session {}. Current count: {}, max_iterations: {}.",
            request.session_id, request.current_count, request.max_iterations
        )),
        headers: None,
        category: None,
        rate_limit_type: Some(RateLimitDimension::MaxIterations),
        model: Some(model),
        llm_provider: Some(llm_provider),
    })
}
