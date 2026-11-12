use super::error::CometApiError;
use super::{DEFAULT_API_BASE, EMBEDDING_ENDPOINT};
use crate::openai_base_config::Headers;
use betterlite_config::sections::cometapi_embedding_credentials::Settings;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::error::Category;
use std::str;
pub const MAX_REPLY_BYTES: usize = TransportLimits::DEFAULT.body_bytes;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CometEmbeddingReply {
    pub model: Option<Json>,
    pub data: Option<Json>,
    pub object: Option<Json>,
    pub usage: CometEmbeddingUsage,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CometEmbeddingUsage {
    pub prompt_tokens: i64,
    pub total_tokens: i64,
}
// an absent base takes the default, while any given one is trimmed and still gains /embeddings
#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    let base = api_base.map_or(DEFAULT_API_BASE, |base| base.trim_end_matches('/'));
    format!("{base}/{EMBEDDING_ENDPOINT}")
}
// an absent caller key falls back to COMETAPI_KEY, and a missing one still writes Bearer None
#[must_use]
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    environment: &Settings,
) -> Headers {
    let key = api_key.or(environment.api_key.as_ref());
    let mut merged = Headers::new();
    merged.insert(
        String::from("Authorization"),
        key.map_or_else(
            || String::from("Bearer None"),
            |secret| format!("Bearer {}", secret.expose_secret()),
        ),
    );
    merged.insert(String::from("accept"), String::from("application/json"));
    merged.insert(
        String::from("Content-Type"),
        String::from("application/json"),
    );
    merged.extend(headers.clone());
    merged
}
// the embedding adapter copies only dimensions, encoding_format and user, whatever drop_params says
pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    for (name, value) in non_default_params {
        if ["dimensions", "encoding_format", "user"].contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}
// options sit over input and model, so an option with either name wins exactly as the dict merge does
#[must_use]
pub fn transform_embedding_request(model: &str, input: &Json, optional_params: &Object) -> Object {
    let mut body = Object::from([
        (String::from("input"), input.clone()),
        (String::from("model"), Json::Text(model.to_owned())),
    ]);
    body.extend(optional_params.clone());
    body
}
// the three passthrough fields and the counters are read as a typed record, where an absent or null part takes its default
#[derive(Debug, Deserialize)]
struct WireReply {
    #[serde(default)]
    model: Option<Json>,
    #[serde(default)]
    data: Option<Json>,
    #[serde(default)]
    object: Option<Json>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Debug, Default, Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: Option<i64>,
    #[serde(default)]
    total_tokens: Option<i64>,
}

// model, data and object are copied with a nullable get; a missing usage contributes zero counters
pub fn decode_comet_embedding_reply(
    body: &[u8],
    status: u16,
    headers: &Headers,
) -> Result<CometEmbeddingReply, CometApiError> {
    if body.len() > MAX_REPLY_BYTES {
        return Err(CometApiError {
            message: format!(
                "the CometAPI embedding reply is {} bytes, over the {MAX_REPLY_BYTES} byte limit",
                body.len()
            ),
            status_code: None,
            headers: Headers::new(),
        });
    }
    let reply = serde_json::from_slice::<WireReply>(body).map_err(|source| {
        if source.classify() == Category::Data {
            return CometApiError {
                message: format!("the CometAPI embedding reply is malformed: {source}"),
                status_code: None,
                headers: Headers::new(),
            };
        }
        CometApiError {
            message: str::from_utf8(body).map_or_else(
                |_not_utf8| String::from("the CometAPI embedding reply is not valid JSON"),
                str::to_owned,
            ),
            status_code: Some(status),
            headers: headers.clone(),
        }
    })?;
    let usage = reply.usage.unwrap_or_default();
    Ok(CometEmbeddingReply {
        model: reply.model,
        data: reply.data,
        object: reply.object,
        usage: CometEmbeddingUsage {
            prompt_tokens: usage.prompt_tokens.unwrap_or_default(),
            total_tokens: usage.total_tokens.unwrap_or_default(),
        },
    })
}
