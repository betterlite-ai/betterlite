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

use std::future::Future;

use self::error::SkillsLoopError;
use self::messages::MessagesContinuation;
use self::request::SkillsRequestData;
use super::hook::SkillsInjectionHook;
use super::hook::execution::{SkillFiles, SkillsSandbox};
use super::hook::response::HookResponse;

pub const CODE_EXECUTION_TOOL: &str = "litellm_code_execution";

// The workspace denies multiple_inherent_impl and hook.rs holds the type's one impl block,
// so the continuation's two steps are trait methods that delegate to rounds.rs.
pub trait ContinuationLoop {
    fn post_call_success<C>(
        &self,
        request_data: &SkillsRequestData,
        response: HookResponse,
        continuation: &C,
    ) -> impl Future<Output = Result<Option<HookResponse>, SkillsLoopError>> + Send
    where
        C: MessagesContinuation + Sync;

    fn execute_messages_loop<C>(
        &self,
        data: &SkillsRequestData,
        response: HookResponse,
        skill_files: &SkillFiles,
        continuation: &C,
    ) -> impl Future<Output = Result<HookResponse, SkillsLoopError>> + Send
    where
        C: MessagesContinuation + Sync;
}

impl<PromptHandler, Sandbox> ContinuationLoop for SkillsInjectionHook<PromptHandler, Sandbox>
where
    PromptHandler: Sync,
    Sandbox: SkillsSandbox + Sync,
{
    fn post_call_success<C>(
        &self,
        request_data: &SkillsRequestData,
        response: HookResponse,
        continuation: &C,
    ) -> impl Future<Output = Result<Option<HookResponse>, SkillsLoopError>> + Send
    where
        C: MessagesContinuation + Sync,
    {
        rounds::post_call_success(self, request_data, response, continuation)
    }

    fn execute_messages_loop<C>(
        &self,
        data: &SkillsRequestData,
        response: HookResponse,
        skill_files: &SkillFiles,
        continuation: &C,
    ) -> impl Future<Output = Result<HookResponse, SkillsLoopError>> + Send
    where
        C: MessagesContinuation + Sync,
    {
        rounds::execute_messages_loop(self, data, response, skill_files, continuation)
    }
}
