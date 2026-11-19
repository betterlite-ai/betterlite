use betterlite_config::sections::bedrock_services_search::DEFAULT_MCP_PROTOCOL_VERSION;
use betterlite_openai::openai_base_config::Headers;

pub const ACCEPT: &str = "application/json, text/event-stream";
pub const CONTENT_TYPE: &str = "application/json";

// LiteLLM overlays these exact keys, so a caller header with different casing is a second entry
#[must_use]
pub fn validate_environment(
    mut headers: Headers,
    configured_protocol_version: Option<&str>,
) -> Headers {
    headers.insert("Content-Type".to_owned(), CONTENT_TYPE.to_owned());
    headers.insert("Accept".to_owned(), ACCEPT.to_owned());
    let version = configured_protocol_version
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_MCP_PROTOCOL_VERSION);
    headers.insert("MCP-Protocol-Version".to_owned(), version.to_owned());
    headers
}
