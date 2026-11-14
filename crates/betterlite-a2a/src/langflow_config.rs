use std::collections::BTreeMap;
use std::error::Error;

use betterlite_core::agents::json::AgentJson;
use futures_util::StreamExt as _;
use futures_util::stream::iter;
use thiserror::Error;

use crate::completion_bridge::completion::CompletionPort;
use crate::completion_bridge::dispatch::{
    BridgeEvents, CompletionError, handle_non_streaming_direct, handle_streaming_direct,
};
use crate::completion_bridge::params::{BridgeCall, ParamsError, optional_text};
use crate::completion_bridge::{A2A_USER_API_KEY_HASH_PARAM, EventSource};
use crate::utils::session_scoping::{
    SessionScopeError, get_session_id_from_a2a_params, scope_session_to_principal,
};

mod tests;

#[derive(Debug, Error)]
pub enum LangFlowConfigError {
    #[error(
        "litellm_params is required for LangFlowA2AConfig (must contain custom_llm_provider and model)"
    )]
    MissingLitellmParams,
    #[error(transparent)]
    Session(#[from] SessionScopeError),
    #[error(transparent)]
    Params(#[from] ParamsError),
}

#[derive(Debug, Error)]
pub enum LangFlowError<C>
where
    C: Error + 'static,
{
    #[error(transparent)]
    Config(#[from] LangFlowConfigError),
    #[error(transparent)]
    Completion(#[from] CompletionError<C>),
}

pub async fn handle_non_streaming<C, S>(
    call: &BridgeCall<'_>,
    completion: &C,
    source: &mut S,
) -> Result<AgentJson, LangFlowError<C::Error>>
where
    C: CompletionPort,
    S: EventSource,
{
    let merged = merged_params(call)?;
    handle_non_streaming_direct(&scoped_call(call, &merged), completion, source)
        .await
        .map_err(LangFlowError::Completion)
}

#[must_use]
pub fn handle_streaming<'call, C, S>(
    call: &BridgeCall<'_>,
    completion: &'call C,
    source: S,
) -> BridgeEvents<'call, LangFlowError<C::Error>>
where
    C: CompletionPort,
    S: EventSource + Send + 'static,
{
    let merged = match merged_params(call) {
        Ok(merged) => merged,
        Err(error) => return iter(Some(Err(LangFlowError::Config(error)))).boxed(),
    };
    handle_streaming_direct(&scoped_call(call, &merged), completion, source)
        .map(|event| event.map_err(LangFlowError::Completion))
        .boxed()
}

const fn scoped_call<'call>(
    call: &'call BridgeCall<'_>,
    merged: &'call BTreeMap<String, AgentJson>,
) -> BridgeCall<'call> {
    BridgeCall {
        litellm_params: merged,
        agent_extra_headers: None,
        ..*call
    }
}

fn merged_params(
    call: &BridgeCall<'_>,
) -> Result<BTreeMap<String, AgentJson>, LangFlowConfigError> {
    if call.litellm_params.is_empty() {
        return Err(LangFlowConfigError::MissingLitellmParams);
    }
    let mut merged = call.litellm_params.clone();
    let Some(session_id) = get_session_id_from_a2a_params(call.params)? else {
        return Ok(merged);
    };
    if session_id.is_empty() || merged.contains_key("session_id") {
        return Ok(merged);
    }
    let principal = optional_text(&merged, A2A_USER_API_KEY_HASH_PARAM)?;
    merged.insert(
        "session_id".to_owned(),
        AgentJson::String(scope_session_to_principal(&session_id, principal)),
    );
    Ok(merged)
}
