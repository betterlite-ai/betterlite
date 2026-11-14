use indexmap::IndexMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StudioAgentError {
    #[error("When overriding api_base for Gemini agents, you must also supply an explicit api_key")]
    CustomBaseWithoutKey,
    #[error("Google API key is required. Set GOOGLE_API_KEY or GEMINI_API_KEY, or pass api_key.")]
    MissingApiKey,
    #[error("the agent reply is {size} bytes, over the {limit} byte limit")]
    ResponseTooLarge { size: usize, limit: usize },
    #[error("Gemini answered with status {status}: {body}")]
    Gemini {
        status: u16,
        body: String,
        headers: IndexMap<String, String>,
    },
    #[error("the agent reply field {field} does not have the shape the source expects")]
    Shape { field: &'static str },
}
