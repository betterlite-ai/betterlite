use crate::chat::{ChatMessage, Tool};
use crate::edenai_chat::ReasoningLookup;

pub const PROVIDER: &str = "zai";

const SUPPORTED_PARAMS: [&str; 8] = [
    "max_tokens",
    "stream",
    "stream_options",
    "temperature",
    "top_p",
    "stop",
    "tools",
    "tool_choice",
];
const THINKING: &str = "thinking";

// ZAIChatConfig adds the thinking param and keeps cache_control, which GLM understands
#[derive(Debug, Clone, Copy, Default)]
pub struct ZaiChatConfig;

impl ZaiChatConfig {
    // a failed capability lookup means no thinking and is not an error
    #[must_use]
    pub fn supported_params<Lookup>(lookup: &Lookup, model: &str) -> Vec<&'static str>
    where
        Lookup: ReasoningLookup,
    {
        let mut params = SUPPORTED_PARAMS.to_vec();
        if lookup.supports_reasoning(model).unwrap_or(false) {
            params.push(THINKING);
        }
        params
    }

    // the override GLM needs: messages and tools come back exactly as given, so cache_control survives
    #[must_use]
    // ast-grep-ignore: no-one-line-functions - the zai config keeps this passthrough for its cache_control override
    pub fn keep_cache_control(
        _model: &str,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
    ) -> (Vec<ChatMessage>, Option<Vec<Tool>>) {
        (messages.to_vec(), tools.map(<[Tool]>::to_vec))
    }
}
