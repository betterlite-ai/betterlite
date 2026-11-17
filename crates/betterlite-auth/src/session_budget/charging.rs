use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use crate::agent::AgentRef;
use crate::agent::get_agent_by_id;
use crate::agent_registry::AgentRegistry;

use super::MAX_BUDGET_PER_SESSION;
use super::SuccessEvent;
use super::fields::text_field;

mod tests;

// the session a successful call charges
pub(crate) struct SessionCharge<'input> {
    pub(crate) session_id: &'input str,
    pub(crate) amount: Decimal,
}

// async_log_success_event before the increment: ids come only from kwargs["litellm_params"]["metadata"]
pub(crate) fn charge_for<'input>(
    registry: &AgentRegistry,
    event: &SuccessEvent<'input>,
) -> Option<SessionCharge<'input>> {
    let metadata = metadata_of(event.litellm_params?)?;
    let session_id = text_field(metadata, "session_id")?;
    let agent_id = text_field(metadata, "agent_id")?;
    let agent = get_agent_by_id(registry, agent_id)?;
    if !has_session_budget(&agent) {
        return None;
    }
    let amount = event.response_cost?;
    if amount <= Decimal::ZERO {
        return None;
    }
    Some(SessionCharge { session_id, amount })
}

// litellm_params.get("metadata") or {}: a missing or non-object metadata is an empty one
fn metadata_of(params: &Json) -> Option<&Json> {
    let Json::Object(fields) = params else {
        return None;
    };
    let metadata = fields.get("metadata")?;
    let Json::Object(_) = metadata else {
        return None;
    };
    Some(metadata)
}

// agent_litellm_params.get("max_budget_per_session") is not None: the configured value is not read here
fn has_session_budget(agent: &AgentRef) -> bool {
    let Some(raw) = agent.litellm_params.get(MAX_BUDGET_PER_SESSION) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Json>(raw.get()) else {
        return false;
    };
    !matches!(value, Json::Null)
}
