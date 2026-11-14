pub mod entry;
pub mod error;
pub mod fake_stream;
mod followup;
pub mod safety;
pub mod settings;

// shared fakes and recording ports for the loop's module tests
pub(crate) mod tests;

use std::error::Error;

use async_trait::async_trait;
use betterlite_context::agentic_callback::AgenticLogging;
use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
use betterlite_openai::stream::chunk::Chunk;
use futures_util::stream::BoxStream;
use serde_json::value::RawValue;

// LiteLLM's wire names for the loop, shared by the modules and their tests
pub(crate) const AGENTIC_LOOP_API_SURFACE_KEY: &str = "_agentic_loop_api_surface";
pub(crate) const AGENTIC_LOOP_DEPTH_KEY: &str = "_agentic_loop_depth";
pub(crate) const AGENTIC_LOOP_FINGERPRINTS_KEY: &str = "_agentic_loop_fingerprints";
pub(crate) const AGENTIC_LOOP_PREFIX: &str = "_agentic_loop";
pub(crate) const CHAT_COMPLETIONS_SURFACE: &str = "chat_completions";
pub(crate) const CUSTOM_LLM_PROVIDER_KEY: &str = "custom_llm_provider";
pub(crate) const HEADROOM_CONVERTED_STREAM_KEY: &str = "_headroom_interception_converted_stream";
pub(crate) const LITELLM_METADATA_KEY: &str = "litellm_metadata";
pub(crate) const MAX_AGENTIC_LOOPS_KEY: &str = "max_agentic_loops";
pub(crate) const MESSAGES_KEY: &str = "messages";
pub(crate) const MODEL_KEY: &str = "model";
pub(crate) const TOOL_CHOICE_KEY: &str = "tool_choice";
pub(crate) const TOOLS_KEY: &str = "tools";
pub(crate) const UNKNOWN_CALL_ID: &str = "unknown";

pub enum ChatLoopResult {
    Response(Box<RawValue>),
    Stream(BoxStream<'static, Chunk>),
}

// the recursive completion call the family composition above this module supplies
#[async_trait]
pub trait ChatContinuation: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn complete(
        &self,
        model: &str,
        messages: &[FollowupMessage],
        params: &JsonMap,
        kwargs: &JsonMap,
    ) -> Result<ChatLoopResult, Self::Error>;
}

// one loop run's inputs, bundled because the call needs six of them plus the ports
pub struct ChatLoopCall<'call> {
    pub response: &'call RawValue,
    pub model: &'call str,
    pub messages: &'call [FollowupMessage],
    pub optional_params: &'call JsonMap,
    pub kwargs: &'call JsonMap,
    pub logging: LoggingHandle<'call>,
    pub custom_llm_provider: &'call str,
    pub stream: bool,
}

pub struct LoopPorts<'call, Continuation: ?Sized, FakeStream: ?Sized> {
    pub continuation: &'call Continuation,
    pub fake_stream: &'call FakeStream,
}

// LiteLLM's logging_obj check: only a real logging object turns a response into a fake stream
#[derive(Clone, Copy, Debug)]
pub enum LoggingHandle<'call> {
    Object(&'call AgenticLogging),
    Absent,
}

impl LoggingHandle<'_> {
    pub(crate) const fn agentic(&self) -> Option<&AgenticLogging> {
        match self {
            Self::Object(logging) => Some(*logging),
            Self::Absent => None,
        }
    }

    pub(crate) fn call_id(&self) -> &str {
        match self {
            Self::Object(logging) => logging.call_id.as_deref().unwrap_or(UNKNOWN_CALL_ID),
            Self::Absent => UNKNOWN_CALL_ID,
        }
    }
}
