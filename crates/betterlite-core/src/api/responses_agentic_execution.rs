pub mod anthropic_plan;
pub mod dispatch;
pub mod error;
pub mod executor;
pub mod fake_stream;
pub mod finalize;
pub mod hooks;
pub mod http;
pub mod responses_plan;
pub mod values;

mod tests;

use betterlite_context::agentic_loop_plan::JsonMap;
use serde_json::value::RawValue;

use crate::api::messages_fake_stream::MessagesFakeStream;

// LiteLLM's wire names for the loop, shared by the modules and their tests
pub(crate) const AGENTIC_LOOP_API_SURFACE_KEY: &str = "_agentic_loop_api_surface";
pub(crate) const AGENTIC_LOOP_DEPTH_KEY: &str = "_agentic_loop_depth";
pub(crate) const AGENTIC_LOOP_FINGERPRINTS_KEY: &str = "_agentic_loop_fingerprints";
pub(crate) const CONVERTED_STREAM_KEY: &str = "_code_interpreter_interception_converted_stream";
pub(crate) const CUSTOM_LLM_PROVIDER_KEY: &str = "custom_llm_provider";
pub(crate) const DEFAULT_MAX_TOKENS: u64 = 1024;
pub(crate) const INPUT_KEY: &str = "input";
pub(crate) const LITELLM_LOGGING_OBJ_KEY: &str = "litellm_logging_obj";
pub(crate) const MAX_AGENTIC_LOOPS_KEY: &str = "max_agentic_loops";
pub(crate) const MAX_TOKENS_KEY: &str = "max_tokens";
pub(crate) const MESSAGES_KEY: &str = "messages";
pub(crate) const MODEL_KEY: &str = "model";
pub(crate) const STREAM_KEY: &str = "stream";
pub(crate) const TOOLS_KEY: &str = "tools";
pub(crate) const UNKNOWN_CALL_ID: &str = "unknown";

// LiteLLM reads the call id, the recorded agentic params and the converted-stream flag off logging_obj
#[derive(Clone, Copy, Debug)]
pub struct LoggingView<'log> {
    pub call_id: Option<&'log str>,
    pub agentic_loop_params: Option<&'log JsonMap>,
    pub websearch_converted_stream: bool,
}

// getattr(logging_obj, "litellm_call_id", "unknown"), with the logging object optional as well
pub(crate) const fn logged_call_id(logging: Option<LoggingView<'_>>) -> &str {
    match logging {
        Some(view) => match view.call_id {
            Some(call_id) => call_id,
            None => UNKNOWN_CALL_ID,
        },
        None => UNKNOWN_CALL_ID,
    }
}

// the loop's answer: the response it settled on, or the B978 stream built from it
pub enum AgenticResult {
    Response(Box<RawValue>),
    Stream(MessagesFakeStream),
}
