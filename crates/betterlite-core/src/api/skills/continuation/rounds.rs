use serde_json::Value;

use super::CODE_EXECUTION_TOOL;
use super::content::{assistant_message, message, stop_reason, tool_result};
use super::error::SkillsLoopError;
use super::messages::{ContinuationRequest, MessagesContinuation};
use super::request::SkillsRequestData;
use crate::api::skills::hook::SkillsInjectionHook;
use crate::api::skills::hook::execution::{SkillFiles, SkillsSandbox};
use crate::api::skills::hook::response::{GeneratedFile, HookResponse, attach_files_to_response};
use crate::api::skills::hook::settings::LITELLM_SKILL_ID_PREFIX;
use crate::api::skills::hook::tool_calls::{ExtractedToolCall, extract_tool_calls};

// The loop's own literal in LiteLLM, independent of its DEFAULT_MAX_TOKENS setting
const DEFAULT_MAX_TOKENS: u64 = 4096;

impl<PromptHandler: Sync, Sandbox: SkillsSandbox + Sync>
    SkillsInjectionHook<PromptHandler, Sandbox>
{
    pub async fn execute_messages_loop<C: MessagesContinuation + Sync>(
        &self,
        data: &SkillsRequestData,
        response: HookResponse,
        skill_files: &SkillFiles,
        continuation: &C,
    ) -> Result<HookResponse, SkillsLoopError> {
        let mut messages = data.messages.clone();
        let mut generated_files = Vec::new();
        let mut current_response = response.clone();
        for round in 1..=self.settings.max_iterations {
            let tool_calls = extract_tool_calls(&current_response)?;
            let finished = stop_reason(&current_response) != Some("tool_use");
            messages.push(assistant_message(&current_response));
            if finished || tool_calls.is_empty() {
                tracing::debug!(
                    "SkillsInjectionHook: Loop completed after {round} iterations, {} files generated",
                    generated_files.len()
                );
                return Ok(attach_files_to_response(current_response, &generated_files));
            }
            let tool_results = self
                .run_tool_calls(&tool_calls, skill_files, &mut generated_files)
                .await;
            messages.push(message("user", tool_results));
            tracing::debug!(
                "SkillsInjectionHook: Making LLM call iteration {}",
                round.saturating_add(1)
            );
            let request = ContinuationRequest {
                model: data.model.clone(),
                messages: messages.clone(),
                tools: data.tools.clone(),
                max_tokens: data.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            };
            let Some(next) = next_response(continuation, &request).await else {
                return Ok(attach_files_to_response(response, &generated_files));
            };
            current_response = next;
        }
        tracing::warn!(
            "SkillsInjectionHook: Max iterations ({}) reached",
            self.settings.max_iterations
        );
        Ok(attach_files_to_response(current_response, &generated_files))
    }

    async fn run_tool_calls(
        &self,
        tool_calls: &[ExtractedToolCall],
        skill_files: &SkillFiles,
        generated_files: &mut Vec<GeneratedFile>,
    ) -> Vec<Value> {
        let mut tool_results = Vec::with_capacity(tool_calls.len());
        for call in tool_calls {
            let name = call.name.as_deref().unwrap_or_default();
            let result = if name == CODE_EXECUTION_TOOL {
                let code = call.input.get("code").and_then(Value::as_str);
                self.execute_code(code.unwrap_or_default(), skill_files, generated_files)
                    .await
            } else if name.starts_with(LITELLM_SKILL_ID_PREFIX) {
                self.execute_skill_tool(name, &call.input, skill_files, generated_files)
                    .await
            } else {
                format!("Tool '{name}' not handled")
            };
            tool_results.push(tool_result(call.id.as_deref(), result));
        }
        tool_results
    }
}

async fn next_response<C: MessagesContinuation + Sync>(
    continuation: &C,
    request: &ContinuationRequest,
) -> Option<HookResponse> {
    match continuation.create(request).await {
        Ok(Some(next)) => Some(next),
        Ok(None) => {
            tracing::error!("SkillsInjectionHook: LLM call returned None");
            None
        }
        Err(error) => {
            tracing::error!("SkillsInjectionHook: LLM call failed: {error}");
            None
        }
    }
}
