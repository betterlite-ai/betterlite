use crate::capabilities::capability_struct;
use serde::{Deserialize, Serialize};

capability_struct! {
    #[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    pub struct ToolingCapabilities {
        supports_function_calling: "supports_function_calling",
        supports_parallel_function_calling: "supports_parallel_function_calling",
        supports_forced_tool_use: "supports_forced_tool_use",
        supports_tool_choice: "supports_tool_choice",
        supports_tool_search: "supports_tool_search",
        supports_computer_use: "supports_computer_use",
        supports_web_search: "supports_web_search",
        supports_url_context: "supports_url_context",
        supports_response_schema: "supports_response_schema",
        supports_native_structured_output: "supports_native_structured_output",
        supports_sampling_params: "supports_sampling_params",
        supports_assistant_prefill: "supports_assistant_prefill",
        supports_system_messages: "supports_system_messages",
        supports_mid_conversation_system: "supports_mid_conversation_system",
        supports_prompt_caching: "supports_prompt_caching",
        supports_prompt_cache_breakpoint: "supports_prompt_cache_breakpoint",
        supports_native_streaming: "supports_native_streaming",
        supports_speed: "supports_speed",
        supports_fast_mode: "supports_fast_mode",
    }
}
