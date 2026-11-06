use thiserror::Error;
use url::Url;

use super::model_fallback::{Provider, ResolvedModel};

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

pub fn destination(api_base: &str, model: &ResolvedModel) -> Result<Destination, RoutingError> {
    let (path, provider_url) = match model.provider {
        Provider::OpenAi => ("oai/v1/log", OPENAI_URL),
        Provider::Claude => ("anthropic/v1/log", ANTHROPIC_URL),
        Provider::VertexAi => ("custom/v1/log", VERTEX_URL),
        Provider::Gemini => ("custom/v1/log", GEMINI_URL),
    };
    let log_url = Url::parse(&format!("{api_base}/{path}")).map_err(RoutingError::InvalidUrl)?;
    Ok(Destination {
        log_url,
        provider_url,
    })
}

mod tests;
