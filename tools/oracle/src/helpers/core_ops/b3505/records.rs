use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::AgentCreateResponse;
use betterlite_http::interactions::base::{Params, RequestHeaders, UpstreamResponse};
use betterlite_models::controls::json_value::JsonValue;
use secrecy::SecretString;

use super::fixture::FixtureError;

#[derive(Default, serde::Deserialize)]
struct AgentWire {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, AgentJson>,
}

#[derive(Default, serde::Deserialize)]
pub(super) struct VersionsWire {
    #[serde(default, rename = "agentVersions")]
    pub(super) agent_versions: Vec<BTreeMap<String, AgentJson>>,
    #[serde(default, rename = "nextPageToken")]
    pub(super) next_page_token: Option<String>,
}

// LiteLLM normalises Gemini's "id" into both fields, and a body without JSON falls back to the name
pub(super) fn agent_response(
    response: &UpstreamResponse,
    name: &str,
) -> Result<AgentCreateResponse, FixtureError> {
    ensure_success(response)?;
    let wire: AgentWire = serde_json::from_slice(&response.body).unwrap_or_default();
    let id = wire.id.unwrap_or_else(|| name.to_owned());
    let surface = wire.name.unwrap_or_else(|| id.clone());
    Ok(AgentCreateResponse {
        id: Some(id),
        name: Some(surface),
        extra: wire.extra,
        hidden_params: BTreeMap::new(),
    })
}

pub(super) fn ensure_success(response: &UpstreamResponse) -> Result<(), FixtureError> {
    if (200..300).contains(&response.status_code) {
        return Ok(());
    }
    Err(FixtureError {
        status: response.status_code,
        message: String::from_utf8_lossy(&response.body).into_owned(),
    })
}

pub(super) fn gemini_headers(
    extra: &BTreeMap<String, String>,
    litellm_params: &Params,
) -> RequestHeaders {
    let mut headers: RequestHeaders = extra
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect();
    headers.insert(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    );
    if let Some(JsonValue::Text(key)) = litellm_params.get("api_key") {
        headers.insert("x-goog-api-key".to_owned(), SecretString::from(key.clone()));
    }
    headers
}

// both pagination keys travel as text; a value of any other type is left out of the query
pub(super) fn page_params(params: &Params) -> Params {
    let mut query = Params::new();
    for (key, name) in [("page_size", "pageSize"), ("page_token", "pageToken")] {
        let text = match params.get(key) {
            Some(JsonValue::Text(text)) => text.clone(),
            Some(JsonValue::Number(number)) => number.to_string(),
            Some(
                JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_),
            )
            | None => continue,
        };
        query.insert(name.to_owned(), JsonValue::Text(text));
    }
    query
}
