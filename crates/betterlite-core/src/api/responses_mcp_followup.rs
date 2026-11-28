pub mod error;
pub mod events;
pub mod output;
pub mod params;
pub mod records;
pub mod streaming;

mod tests;

use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;

use crate::api::responses_mcp_events::event::StreamEvent;
use crate::api::responses_provider_selection::ParameterMap;

use self::error::McpFollowupError;
use self::events::{CallEventsBuilder, EventToolResult, create_tool_execution_events};
use self::output::append_mcp_output_items;
use self::params::{follow_up_call, prepare_follow_up_call_params, prepare_initial_call_params};
use self::records::{
    FollowUpCall, IdSource, McpStreamingPlan, ResponsesMcpFollowupInput, ResponsesMcpFollowupResult,
};
use self::streaming::{StreamingRequest, create_mcp_streaming_response};

// uuid.uuid4().hex[:8] (litellm_proxy_mcp_handler.py:1267, L1326): each generated id keeps its first 8 characters
const SHORT_ID_LENGTH: usize = 8;

// aresponses as _make_follow_up_call (L1112-L1118) awaits it
#[async_trait]
pub trait FollowUpProvider: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn call(&self, call: &FollowUpCall) -> Result<ResponsesApiResponse, Self::Error>;
}

pub struct ResponsesMcpFollowup<'deps, Provider> {
    pub provider: &'deps Provider,
    pub ids: IdSource,
    pub event_builder: &'deps dyn CallEventsBuilder,
    pub should_auto_execute: bool,
    pub original_stream: bool,
}

struct Prepared {
    initial_params: ParameterMap,
    follow_up_params: ParameterMap,
    call: FollowUpCall,
}

impl Prepared {
    fn into_result(
        self,
        streaming: Option<McpStreamingPlan>,
        events: Vec<StreamEvent>,
        response: Option<ResponsesApiResponse>,
    ) -> ResponsesMcpFollowupResult {
        ResponsesMcpFollowupResult {
            initial_params: self.initial_params,
            follow_up_params: self.follow_up_params,
            call: self.call,
            streaming,
            events,
            response,
        }
    }
}

impl<Provider> ResponsesMcpFollowup<'_, Provider>
where
    Provider: FollowUpProvider,
{
    pub async fn run(
        &self,
        input: &ResponsesMcpFollowupInput,
    ) -> Result<ResponsesMcpFollowupResult, McpFollowupError<Provider::Error>> {
        let prepared = self.prepare(input);
        if self.original_stream {
            return self.streaming(input, prepared);
        }
        self.completed(input, prepared).await
    }

    fn prepare(&self, input: &ResponsesMcpFollowupInput) -> Prepared {
        let follow_up_params =
            prepare_follow_up_call_params(&input.call_params, self.original_stream);
        Prepared {
            initial_params: prepare_initial_call_params(
                &input.call_params,
                self.should_auto_execute,
            ),
            call: follow_up_call(
                &input.follow_up_input,
                &input.model,
                input.tools.as_deref(),
                input.follow_up_response_id.as_deref(),
                &follow_up_params,
            ),
            follow_up_params,
        }
    }

    fn streaming(
        &self,
        input: &ResponsesMcpFollowupInput,
        prepared: Prepared,
    ) -> Result<ResponsesMcpFollowupResult, McpFollowupError<Provider::Error>> {
        let plan = create_mcp_streaming_response(&StreamingRequest {
            input: &input.input,
            model: &input.model,
            tools: input.tools.as_deref(),
            previous_response_id: input.previous_response_id.as_deref(),
            call_params: &input.call_params,
            kwargs: &input.kwargs,
            mcp_tools_with_litellm_proxy: &input.mcp_tools_with_litellm_proxy,
            mcp_discovery_events: &input.mcp_discovery_events,
            tool_server_map: &input.tool_server_map,
            auth: &input.auth,
        })?;
        let results: Vec<EventToolResult> = input
            .tool_results
            .iter()
            .map(EventToolResult::from)
            .collect();
        let events = create_tool_execution_events(
            self.event_builder,
            &input.tool_calls,
            &results,
            &self.ids,
        )?;
        Ok(prepared.into_result(Some(plan), events, None))
    }

    async fn completed(
        &self,
        input: &ResponsesMcpFollowupInput,
        prepared: Prepared,
    ) -> Result<ResponsesMcpFollowupResult, McpFollowupError<Provider::Error>> {
        let mut response = self
            .provider
            .call(&prepared.call)
            .await
            .map_err(McpFollowupError::Provider)?;
        append_mcp_output_items(
            &mut response,
            &input.mcp_tools_fetched,
            &input.tool_results,
            &self.ids,
        );
        Ok(prepared.into_result(None, Vec::new(), Some(response)))
    }
}
