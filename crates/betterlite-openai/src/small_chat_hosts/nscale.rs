pub const PROVIDER: &str = "nscale";

const SUPPORTED_PARAMS: [&str; 12] = [
    "max_tokens",
    "n",
    "temperature",
    "top_p",
    "stream",
    "logprobs",
    "top_logprobs",
    "frequency_penalty",
    "presence_penalty",
    "response_format",
    "stop",
    "logit_bias",
];

// NscaleConfig redefines only its supported list; the rest is OpenAIGPTConfig
#[derive(Debug, Clone, Copy, Default)]
pub struct NscaleConfig;

impl NscaleConfig {
    #[must_use]
    pub fn supported_params(_model: &str) -> Vec<&'static str> {
        SUPPORTED_PARAMS.to_vec()
    }
}
