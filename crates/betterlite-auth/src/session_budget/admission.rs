use betterlite_error::rate_limit::ProxyRateLimitError;
use betterlite_error::rate_limit::ProxyRateLimitErrorInit;
use betterlite_error::rate_limit::RateLimitDimension;
use betterlite_models::decisions::Json;
use betterlite_redact::structured::Payload;
use rust_decimal::Decimal;

use crate::agent::get_agent_by_id;
use crate::agent_registry::AgentRegistry;
use crate::budget_counter::coercion::FloatInput as _;
use crate::model_budget_values::amount_text::amount_from_text;
use crate::provider_resolution::resolve_rate_limit_provider;
use crate::rejection_provider::DeploymentSnapshot;

use super::MAX_BUDGET_PER_SESSION;
use super::SessionBudgetHandler;
use super::cache::SessionBudgetCache;
use super::error::SessionBudgetError;
use super::fields::text_field;

mod tests;

// the fields the 429 body and its model/provider pair are built from
pub(crate) struct ExceededRequest<'input> {
    pub(crate) session_id: &'input str,
    pub(crate) current_spend: Decimal,
    pub(crate) max_budget: Decimal,
    pub(crate) data: &'input Json,
}

// _get_max_budget_per_session: the agent's litellm_params entry, float()ed, and None when either is missing
pub(crate) fn max_budget_for(
    registry: &AgentRegistry,
    agent_id: Option<&str>,
) -> Result<Option<Decimal>, SessionBudgetError> {
    let Some(agent_id) = agent_id else {
        return Ok(None);
    };
    let Some(agent) = get_agent_by_id(registry, agent_id) else {
        return Ok(None);
    };
    let Some(raw) = agent.litellm_params.get(MAX_BUDGET_PER_SESSION) else {
        return Ok(None);
    };
    let value: Json =
        serde_json::from_str(raw.get()).map_err(|_error| SessionBudgetError::InvalidBudget {
            agent_id: agent_id.to_owned(),
        })?;
    budget_amount(&value, agent_id)
}

// float() on the configured value: a number or numeric text is the budget, and null means no budget
fn budget_amount(value: &Json, agent_id: &str) -> Result<Option<Decimal>, SessionBudgetError> {
    let invalid = || SessionBudgetError::InvalidBudget {
        agent_id: agent_id.to_owned(),
    };
    match value {
        Json::Null => Ok(None),
        Json::Bool(flag) => Ok(Some(if *flag { Decimal::ONE } else { Decimal::ZERO })),
        Json::Number(number) => amount_from_text(&number.to_string())
            .map(Some)
            .ok_or_else(invalid),
        Json::Text(text) => text.as_str().decimal_value().map(Some).ok_or_else(invalid),
        Json::List(_) | Json::Object(_) => Err(invalid()),
    }
}

// _get_session_id: metadata first, then litellm_metadata, and no fallback past a text id it finds
pub(crate) fn session_id_from(data: &Json) -> Option<&str> {
    let Json::Object(fields) = data else {
        return None;
    };
    let metadata = fields
        .get("metadata")
        .and_then(|bucket| text_field(bucket, "session_id"));
    let litellm_metadata = fields
        .get("litellm_metadata")
        .and_then(|bucket| text_field(bucket, "session_id"));
    metadata.or(litellm_metadata)
}

// data.get("model"): the model the rejected request named, when it is a string
fn requested_model(data: &Json) -> Option<&str> {
    let Json::Object(fields) = data else {
        return None;
    };
    let Some(Json::Text(model)) = fields.get("model") else {
        return None;
    };
    Some(model.as_str())
}

// the 429 async_pre_call_hook raises: RateLimitType.BUDGET with the resolved model and provider
pub(crate) fn exceeded_error<C: SessionBudgetCache>(
    handler: &SessionBudgetHandler<C>,
    router: Option<&dyn DeploymentSnapshot>,
    request: &ExceededRequest<'_>,
) -> SessionBudgetError {
    let (model, llm_provider) = resolve_rate_limit_provider(
        requested_model(request.data),
        handler.resolver.as_ref(),
        router,
    );
    let detail = format!(
        "Session budget exceeded for session {}. Current spend: ${:.4}, max_budget_per_session: ${:.2}.",
        request.session_id,
        request.current_spend.round_dp(4),
        request.max_budget.round_dp(2)
    );
    let init = ProxyRateLimitErrorInit {
        detail: Payload::String(detail),
        headers: None,
        category: None,
        rate_limit_type: Some(RateLimitDimension::Budget),
        model: Some(model),
        llm_provider: Some(llm_provider),
    };
    match ProxyRateLimitError::new(init) {
        Ok(error) => SessionBudgetError::Exceeded(Box::new(error)),
        Err(source) => SessionBudgetError::Rejection(source),
    }
}
