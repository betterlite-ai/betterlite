use crate::capabilities::capability_struct;
use serde::{Deserialize, Serialize};

capability_struct! {
    #[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    pub struct VendorCapabilities {
        bedrock_strict_tools: "bedrock_converse_supports_strict_tools",
        supports_bedrock_rt_format: "supports_bedrock_runtime_chat_completions_response_format",
        bedrock_rt_tools_reasoning: "supports_bedrock_runtime_chat_completions_tools_with_reasoning",
        use_openai_responses_path: "use_openai_responses_path",
        supports_output_config: "supports_output_config",
        supports_parallel_tool_config: "supports_parallel_tool_use_config",
    }
}
