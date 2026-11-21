pub const PROVIDER: &str = "morph";

const SUPPORTED_PARAMS: [&str; 3] = ["messages", "model", "stream"];

// MorphChatConfig extends OpenAILikeChatConfig, so only its supported list differs
#[derive(Debug, Clone, Copy, Default)]
pub struct MorphChatConfig;

impl MorphChatConfig {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions - the config API fixes this name and the list is the whole body
    pub fn supported_params(_model: &str) -> Vec<&'static str> {
        SUPPORTED_PARAMS.to_vec()
    }
}
