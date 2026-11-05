use thiserror::Error;
use url::Url;

use super::model_fallback::ResolvedModel;

const OPENAI_URL: &str = "https://api.openai.com/v1";
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const VERTEX_URL: &str = "https://aiplatform.googleapis.com/v1";
const GEMINI_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("the Helicone log URL is not valid")]
    InvalidUrl(#[source] url::ParseError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    pub log_url: Url,
    pub provider_url: &'static str,
}

// Claude outside Vertex AI goes first, then any Vertex AI model, then Gemini, and the rest is OpenAI.
pub fn destination(api_base: &str, model: &ResolvedModel) -> Result<Destination, RoutingError> {
    let (path, provider_url) = if model.claude_format {
        ("anthropic/v1/log", ANTHROPIC_URL)
    } else if model.vertex_ai {
        ("custom/v1/log", VERTEX_URL)
    } else if model.name.contains("gemini") {
        ("custom/v1/log", GEMINI_URL)
    } else {
        ("oai/v1/log", OPENAI_URL)
    };
    let log_url = Url::parse(&format!("{api_base}/{path}")).map_err(RoutingError::InvalidUrl)?;
    Ok(Destination {
        log_url,
        provider_url,
    })
}

mod tests;
