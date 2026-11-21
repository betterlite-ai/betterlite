use std::collections::BTreeMap;

use super::error::AgentCallerError;
use crate::agent_policy::caller::AgentCaller;
use crate::resolved_key::auth::{AuthInput, ParentOtelSpan, UserAPIKeyAuth};

mod tests;

// the stand-in carries no agent_id, so the shared resolvers never cap it as an agent key
pub fn agent_caller_auth(
    auth: &UserAPIKeyAuth,
) -> Result<Option<UserAPIKeyAuth>, AgentCallerError> {
    let Some(caller) = auth.extra.agent_caller.as_ref() else {
        return Ok(None);
    };
    let parent_otel_span = auth.extra.parent_otel_span.clone();
    caller_record(caller, auth.extra.requires_fresh_policy, parent_otel_span).map(Some)
}

pub fn caller_record(
    caller: &AgentCaller,
    requires_fresh_policy: bool,
    parent_otel_span: Option<ParentOtelSpan>,
) -> Result<UserAPIKeyAuth, AgentCallerError> {
    let mut record = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new()))
        .map_err(AgentCallerError::Record)?;
    record.view.token.user_id.clone_from(&caller.user_id);
    record.view.token.team_id.clone_from(&caller.team_id);
    record.extra.parent_otel_span = parent_otel_span;
    // LiteLLM sets it with model_copy, past the validator that strips it from input
    record.extra.requires_fresh_policy = requires_fresh_policy;
    Ok(record)
}
