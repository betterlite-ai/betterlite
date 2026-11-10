use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::count_tokens::error::CountTokensError;

pub type CountResponse = IndexMap<String, Json>;

const COUNT_TOKEN_OPTION_NAMES: [&str; 3] = ["thinking", "tool_choice", "output_config"];

#[derive(Debug)]
pub struct CountTokensBody<'call> {
    pub model: &'call str,
    pub messages: &'call Json,
    pub tools: Option<&'call Json>,
    pub system: Option<&'call Json>,
    pub optional_params: &'call IndexMap<String, Json>,
}

#[derive(Debug)]
pub struct CountTokensRequest<'call> {
    pub body: CountTokensBody<'call>,
    pub api_key: &'call SecretString,
    pub api_base: Option<&'call str>,
    pub timeout: Option<Duration>,
}

#[must_use]
pub fn transform_request(body: &CountTokensBody<'_>) -> IndexMap<String, Json> {
    let mut request = IndexMap::new();
    request.insert("model".to_owned(), Json::Text(body.model.to_owned()));
    request.insert("messages".to_owned(), body.messages.clone());
    if let Some(system) = body.system {
        request.insert("system".to_owned(), system.clone());
    }
    if let Some(tools) = body.tools {
        request.insert("tools".to_owned(), tools.clone());
    }
    for name in COUNT_TOKEN_OPTION_NAMES {
        if let Some(value) = body.optional_params.get(name) {
            request.insert(name.to_owned(), value.clone());
        }
    }
    request
}

pub fn decode_count_response(bytes: &[u8]) -> Result<CountResponse, CountTokensError> {
    let value: Json =
        serde_json::from_slice(bytes).map_err(|source| CountTokensError::Decode { source })?;
    match value {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(CountTokensError::NotAnObject)
        }
    }
}
