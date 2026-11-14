use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;

use super::call::{AgentCoreHandler, HandlerCall, StreamedEvents};
use super::params::LitellmParams;
use super::runtime::AgentCoreRuntime;
use super::{AgentCoreError, TransformError};
use crate::completion_bridge::EventSource;
use crate::completion_bridge::wire::MessageSendParams;

pub struct ConfigCall<'call> {
    pub request_id: &'call str,
    pub params: &'call MessageSendParams,
    // LiteLLM's config methods take api_base and never pass it on: the URL comes from the model ARN
    pub api_base: Option<&'call str>,
    pub litellm_params: Option<&'call BTreeMap<String, AgentJson>>,
    pub agent_extra_headers: Option<&'call BTreeMap<String, String>>,
}

fn handler_call<'call>(call: &'call ConfigCall<'_>) -> Result<HandlerCall<'call>, TransformError> {
    let raw = call
        .litellm_params
        .filter(|raw| !raw.is_empty())
        .ok_or(TransformError::MissingParams)?;
    Ok(HandlerCall {
        request_id: call.request_id,
        params: call.params,
        litellm_params: LitellmParams::parse(raw)?,
        agent_extra_headers: call.agent_extra_headers,
    })
}

pub struct BedrockAgentCoreConfig<R, S> {
    pub handler: AgentCoreHandler<R, S>,
}

impl<R, S> BedrockAgentCoreConfig<R, S>
where
    R: AgentCoreRuntime,
    S: EventSource + Send,
{
    pub async fn handle_non_streaming(
        &mut self,
        call: &ConfigCall<'_>,
    ) -> Result<AgentJson, AgentCoreError<R::Error>> {
        self.handler
            .handle_non_streaming(&handler_call(call)?)
            .await
    }

    pub async fn handle_streaming(
        &mut self,
        call: &ConfigCall<'_>,
    ) -> Result<StreamedEvents<R::Error>, AgentCoreError<R::Error>> {
        self.handler.handle_streaming(&handler_call(call)?).await
    }
}
