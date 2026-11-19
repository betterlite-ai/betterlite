use thiserror::Error;

#[derive(Debug, Error)]
pub enum JsonProviderPolicyError {
    #[error("temperature must be a number to apply the provider's constraints")]
    TemperatureNotANumber,
    #[error("n must be a number to apply the provider's temperature constraint")]
    NNotANumber,
    #[error("api_base is required for openai_like provider")]
    OpenAiLikeBaseRequired,
    #[error("a chat message could not be converted for the prompt helpers: {source}")]
    Message {
        #[source]
        source: serde_json::Error,
    },
}
