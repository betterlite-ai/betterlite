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
        let litellm_params =
            LitellmParams::parse(call.litellm_params.ok_or(TransformError::MissingParams)?)?;
        self.handler
            .handle_non_streaming(&HandlerCall {
                request_id: call.request_id,
                params: call.params,
                litellm_params: &litellm_params,
                agent_extra_headers: call.agent_extra_headers,
            })
            .await
    }

    pub async fn handle_streaming(
        &mut self,
        call: &ConfigCall<'_>,
    ) -> Result<StreamedEvents<R::Error>, AgentCoreError<R::Error>> {
        let litellm_params =
            LitellmParams::parse(call.litellm_params.ok_or(TransformError::MissingParams)?)?;
        self.handler
            .handle_streaming(&HandlerCall {
                request_id: call.request_id,
                params: call.params,
                litellm_params: &litellm_params,
                agent_extra_headers: call.agent_extra_headers,
            })
            .await
    }
}
