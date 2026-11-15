pub mod anthropic_plan;
pub mod dispatch;
pub mod error;
pub mod executor;
pub mod fake_stream;
pub mod finalize;
pub mod hooks;
pub mod http;
pub mod responses_plan;
mod tests;
pub mod values;
use crate::api::messages_fake_stream::MessagesFakeStream;
use betterlite_context::agentic_loop_plan::JsonMap;
use serde_json::value::RawValue;
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
#[derive(Clone, Copy, Debug)]
pub struct LoggingView<'log> {
    pub call_id: Option<&'log str>,
    pub agentic_loop_params: Option<&'log JsonMap>,
    pub websearch_converted_stream: bool,
}
pub(crate) fn logged_call_id(logging: Option<LoggingView<'_>>) -> &str {
    logging
        .and_then(|view| view.call_id)
        .unwrap_or(UNKNOWN_CALL_ID)
}
pub enum AgenticResult {
    Response(Box<RawValue>),
    Stream(MessagesFakeStream),
}
