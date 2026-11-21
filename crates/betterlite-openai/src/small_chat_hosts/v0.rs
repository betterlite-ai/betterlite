pub const PROVIDER: &str = "v0";

const SUPPORTED_PARAMS: [&str; 5] = ["messages", "model", "stream", "tools", "tool_choice"];

// V0ChatConfig extends OpenAILikeChatConfig, so only its supported list differs
#[derive(Debug, Clone, Copy, Default)]
pub struct V0ChatConfig;

impl V0ChatConfig {
    #[must_use]
    pub fn supported_params(_model: &str) -> Vec<&'static str> {
        SUPPORTED_PARAMS.to_vec()
    }
}
