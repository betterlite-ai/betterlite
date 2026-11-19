use betterlite_http::url_path::encode_segment;

use super::env::{CredentialsInput, resolve_api_base};
use super::error::RagflowError;
use super::parse::{EndpointType, parse_model};

const ENTITY_ID_FIELD: &str = "entity_id";
const API_V1: &str = "/api/v1";
const V1: &str = "/v1";
const CHAT_PATH: &str = "chats_openai";
const AGENT_PATH: &str = "agents_openai";
const CHAT_COMPLETIONS: &str = "/chat/completions";

mod tests;

// LiteLLM's get_complete_url: the base is resolved before the model is parsed, so a missing base wins
pub fn complete_url(input: &CredentialsInput<'_>) -> Result<String, RagflowError> {
    let base = resolve_api_base(input)?.ok_or(RagflowError::ApiBaseRequired)?;
    let parsed = parse_model(input.model)?;
    let encoded = encode_segment(&parsed.entity_id, ENTITY_ID_FIELD)?;
    let endpoint = match parsed.endpoint {
        EndpointType::Chat => CHAT_PATH,
        EndpointType::Agent => AGENT_PATH,
    };
    // /api/v1 is checked first because it ends with /v1, and only one suffix goes
    let trimmed = base.trim_end_matches('/');
    let stripped = trimmed
        .strip_suffix(API_V1)
        .or_else(|| trimmed.strip_suffix(V1))
        .unwrap_or(trimmed);
    Ok(format!(
        "{stripped}{API_V1}/{endpoint}/{encoded}{CHAT_COMPLETIONS}"
    ))
}
