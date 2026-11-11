use std::sync::Arc;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use serde_json::Number;

use super::types::{McpHiddenParams, McpPostCallResponse, McpToolHookCall, McpToolResult};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::dynamic_callbacks::DynamicCallback;
use crate::dynamic_callbacks::DynamicCallbackLists;
use crate::dynamic_callbacks::filters::get_combined_callback_list;
use crate::logging_state::LoggingState;
use crate::registry::CallbackRegistry;
use crate::registry::phases::CallbackPhase;

pub async fn async_post_mcp_tool_call_hook(
    state: &mut LoggingState,
    registry: &CallbackRegistry,
    mut result: McpToolResult,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
) -> McpToolResult {
    let lists = DynamicCallbackLists::from(&state.dynamic_callbacks);
    let global: Vec<DynamicCallback> = registry
        .loggers(CallbackPhase::Success)
        .into_iter()
        .map(DynamicCallback::Logger)
        .collect();
    let callbacks = get_combined_callback_list(lists.success.as_deref(), &global);
    let mut walk = McpToolWalk {
        state,
        hidden_params: McpHiddenParams::default(),
        start_time,
        end_time,
    };
    for callback in &callbacks {
        if let DynamicCallback::Logger(logger) = callback {
            walk.apply(logger, &mut result).await;
        }
    }
    result
}

struct McpToolWalk<'state> {
    state: &'state mut LoggingState,
    hidden_params: McpHiddenParams,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
}

impl McpToolWalk<'_> {
    async fn apply(&mut self, logger: &Arc<dyn CallbackLogger>, result: &mut McpToolResult) {
        let original_content = result.content.clone();
        let original_structured = result.structured_content.clone();
        let mut response = McpPostCallResponse {
            mcp_tool_call_response: original_content.clone(),
            hidden_params: self.hidden_params,
        };
        let outcome = logger
            .async_post_mcp_tool_call_hook(McpToolHookCall {
                kwargs: &self.state.model_call_details,
                response: &mut response,
                original: &mut *result,
                start_time: self.start_time,
                end_time: self.end_time,
            })
            .await;
        let hook_content = self.record_outcome(outcome, response);
        // LiteLLM accepts a structured rewrite only when the live content and the hook content agree
        let structured_rewrite_kept = result.structured_content != original_structured
            && match &hook_content {
                None => true,
                Some(content) => content == &original_content || result.content == *content,
            };
        if let Some(content) = hook_content.filter(|content| content != &original_content) {
            result.content = content;
        }
        if result.content != original_content
            && result.structured_content.is_some()
            && !structured_rewrite_kept
        {
            result.structured_content = None;
            result.is_error = true;
        }
    }

    fn record_outcome(
        &mut self,
        outcome: Result<Option<McpPostCallResponse>, LoggerError>,
        response: McpPostCallResponse,
    ) -> Option<Vec<Json>> {
        match outcome {
            Ok(Some(reply)) => {
                let cost = reply
                    .hidden_params
                    .response_cost
                    .and_then(Number::from_f64)
                    .map_or(Json::Null, Json::Number);
                self.state
                    .model_call_details
                    .insert("response_cost".to_owned(), cost);
                self.hidden_params = reply.hidden_params;
                Some(reply.mcp_tool_call_response)
            }
            Ok(None) => Some(response.mcp_tool_call_response),
            Err(error) => {
                tracing::error!(
                    "LiteLLM.LoggingError: [Non-Blocking] Exception occurred while logging {error}"
                );
                None
            }
        }
    }
}
