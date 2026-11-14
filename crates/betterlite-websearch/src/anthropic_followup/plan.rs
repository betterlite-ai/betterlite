use betterlite_context::agentic_loop_plan::{
    AgenticLoopPlan, AgenticLoopRequestPatch, FollowupMessage, JsonMap, get_text,
};
use serde_json::value::to_raw_value;

use super::AnthropicFollowupError;
use super::patch::{AnthropicPatchRequest, FollowupLogging, build_anthropic_request_patch};
use crate::call_transformation::wire::Call;
use crate::chat_responses_followup::ChatResponsesFollowup;
use crate::chat_responses_followup::chat::FollowupRequest;
use crate::chat_responses_followup::responses::{ResponsesInput, ResponsesRequest};
use crate::loop_eligibility::{CHAT_COMPLETION_SURFACE, RESPONSES_AGENTIC_SURFACE, ToolsDict};
use crate::native_blocks::{NATIVE_BLOCKS_METADATA_KEY, ResponseJson, build_native_result_blocks};
use crate::records::SearchOutcome;

const SURFACE_KEY: &str = "_agentic_loop_api_surface";
const NATIVE_FLAG_KEY: &str = "_websearch_interception_emit_native_blocks";
const WEB_SEARCH_FAILED: &str = "web_search_failed";
const WEBSEARCH_TOOL_TYPE: &str = "websearch";
const ANTHROPIC_FORMAT: &str = "anthropic";

pub struct AnthropicFollowup {
    pub followup: ChatResponsesFollowup,
}

pub struct AnthropicPlanRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [FollowupMessage],
    pub optional_params: &'request JsonMap,
    pub logging: Option<&'request FollowupLogging<'request>>,
    pub kwargs: &'request JsonMap,
}

impl AnthropicFollowup {
    pub async fn build_agentic_loop_plan(
        &self,
        tools: &ToolsDict,
        request: &AnthropicPlanRequest<'_>,
    ) -> Result<AgenticLoopPlan, AnthropicFollowupError> {
        match get_text(request.kwargs, SURFACE_KEY).as_deref() {
            Some(CHAT_COMPLETION_SURFACE) => {
                let followup = FollowupRequest {
                    model: request.model,
                    messages: request.messages,
                    optional_params: request.optional_params,
                    kwargs: request.kwargs,
                };
                let plan = self
                    .followup
                    .build_chat_plan(&tools.tool_calls, tools.response_format, &followup)
                    .await?;
                Ok(plan)
            }
            Some(RESPONSES_AGENTIC_SURFACE) => {
                let input = ResponsesInput::Items(request.messages.to_vec());
                let responses = ResponsesRequest {
                    model: request.model,
                    input: &input,
                    optional_params: request.optional_params,
                    kwargs: request.kwargs,
                };
                let plan = self
                    .followup
                    .build_responses_plan(&tools.tool_calls, &responses)
                    .await?;
                Ok(plan)
            }
            _ => self.build_anthropic_plan(tools, request).await,
        }
    }

    async fn build_anthropic_plan(
        &self,
        tools: &ToolsDict,
        request: &AnthropicPlanRequest<'_>,
    ) -> Result<AgenticLoopPlan, AnthropicFollowupError> {
        let patch_request = AnthropicPatchRequest {
            model: request.model,
            messages: request.messages,
            tool_calls: &tools.tool_calls,
            thinking_blocks: tools.thinking_blocks.as_deref().unwrap_or_default(),
            optional_params: request.optional_params,
            logging: request.logging,
            kwargs: request.kwargs,
        };
        let (patch, outcomes) =
            build_anthropic_request_patch(self.followup.search.as_ref(), &patch_request).await?;
        let mut metadata = anthropic_metadata()?;
        if !native_flag(request.kwargs)? {
            return Ok(running_plan(patch, metadata));
        }
        let blocks = native_blocks(&tools.tool_calls, &outcomes)?;
        metadata.insert(
            NATIVE_BLOCKS_METADATA_KEY.to_owned(),
            to_raw_value(&blocks)?,
        );
        if every_search_failed(&outcomes) {
            return Ok(terminated_plan(metadata));
        }
        Ok(running_plan(patch, metadata))
    }
}

fn anthropic_metadata() -> Result<JsonMap, AnthropicFollowupError> {
    let mut metadata = JsonMap::new();
    metadata.insert("tool_type".to_owned(), to_raw_value(WEBSEARCH_TOOL_TYPE)?);
    metadata.insert(
        "response_format".to_owned(),
        to_raw_value(ANTHROPIC_FORMAT)?,
    );
    Ok(metadata)
}

const fn running_plan(patch: AgenticLoopRequestPatch, metadata: JsonMap) -> AgenticLoopPlan {
    AgenticLoopPlan {
        run_agentic_loop: true,
        request_patch: Some(patch),
        response_override: None,
        terminate: false,
        stop_reason: None,
        metadata,
    }
}

fn terminated_plan(metadata: JsonMap) -> AgenticLoopPlan {
    AgenticLoopPlan {
        run_agentic_loop: false,
        request_patch: None,
        response_override: None,
        terminate: true,
        stop_reason: Some(WEB_SEARCH_FAILED.to_owned()),
        metadata,
    }
}

fn native_flag(kwargs: &JsonMap) -> Result<bool, AnthropicFollowupError> {
    let Some(raw) = kwargs.get(NATIVE_FLAG_KEY) else {
        return Ok(false);
    };
    let flag: Option<bool> =
        serde_json::from_str(raw.get()).map_err(AnthropicFollowupError::InvalidNativeBlocksFlag)?;
    Ok(flag.unwrap_or(false))
}

fn every_search_failed(outcomes: &[SearchOutcome]) -> bool {
    !outcomes.is_empty()
        && outcomes
            .iter()
            .all(|outcome| matches!(outcome, SearchOutcome::Failed(_)))
}

fn native_blocks(
    calls: &[Call],
    outcomes: &[SearchOutcome],
) -> Result<Vec<ResponseJson>, AnthropicFollowupError> {
    let pairs = build_native_result_blocks(calls, outcomes)?;
    let mut blocks = Vec::with_capacity(pairs.len().saturating_mul(2));
    for (tool_use, result) in pairs {
        blocks.push(serde_json::to_value(tool_use)?);
        blocks.push(serde_json::to_value(result)?);
    }
    Ok(blocks)
}
