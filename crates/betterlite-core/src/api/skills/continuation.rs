#![expect(
    clippy::disallowed_types,
    reason = "Message and tool bodies are opaque provider JSON that the loop forwards to the Messages call without modelling every field"
)]

pub mod content;
pub mod error;
pub mod messages;
pub mod request;
pub mod rounds;
mod tests;

use self::error::SkillsLoopError;
use self::messages::MessagesContinuation;
use self::request::SkillsRequestData;
use super::hook::SkillsInjectionHook;
use super::hook::execution::{SkillFiles, SkillsSandbox};
use super::hook::response::HookResponse;
use super::hook::settings::LITELLM_SKILL_ID_PREFIX;
use super::hook::tool_calls::extract_tool_calls;

pub const CODE_EXECUTION_TOOL: &str = "litellm_code_execution";

impl<PromptHandler: Sync, Sandbox: SkillsSandbox + Sync>
    SkillsInjectionHook<PromptHandler, Sandbox>
{
    pub async fn post_call_success<C: MessagesContinuation + Sync>(
        &self,
        request_data: &SkillsRequestData,
        response: HookResponse,
        continuation: &C,
    ) -> Result<Option<HookResponse>, SkillsLoopError> {
        if !request_data.skills.code_execution_enabled {
            return Ok(None);
        }
        let mut skill_files = SkillFiles::new();
        for files in request_data.skills.skill_files.values() {
            skill_files.extend(files.clone());
        }
        if skill_files.is_empty() {
            tracing::warn!("SkillsInjectionHook: No skill files found, cannot execute code");
            return Ok(None);
        }
        let executable = extract_tool_calls(&response)?.iter().any(|call| {
            let name = call.name.as_deref().unwrap_or_default();
            name == CODE_EXECUTION_TOOL || name.starts_with(LITELLM_SKILL_ID_PREFIX)
        });
        if !executable {
            return Ok(None);
        }
        tracing::debug!("SkillsInjectionHook: Detected tool call, starting execution loop");
        self.execute_messages_loop(request_data, response, &skill_files, continuation)
            .await
            .map(Some)
    }
}
