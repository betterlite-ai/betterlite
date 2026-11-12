use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde::Serialize;

use super::error::ResponsesContractError;

pub type Params = BTreeMap<String, Json>;
pub type Headers = BTreeMap<String, String>;

const DEFAULT_INPUT_ITEMS_LIMIT: i64 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRequest {
    pub url: String,
    pub params: Params,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InputItemsOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputItemsQuery {
    pub after: Option<String>,
    pub before: Option<String>,
    pub include: Option<Vec<String>>,
    pub limit: i64,
    pub order: InputItemsOrder,
}

impl Default for InputItemsQuery {
    fn default() -> Self {
        Self {
            after: None,
            before: None,
            include: None,
            limit: DEFAULT_INPUT_ITEMS_LIMIT,
            order: InputItemsOrder::Desc,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRequest {
    pub model: String,
    pub input: Json,
    pub optional_params: Params,
    pub litellm_params: Params,
    pub headers: Headers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdRequest {
    pub response_id: String,
    pub api_base: String,
    pub litellm_params: Params,
    pub headers: Headers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputItemsRequest {
    pub response_id: String,
    pub api_base: String,
    pub litellm_params: Params,
    pub headers: Headers,
    pub query: InputItemsQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactRequest {
    pub model: String,
    pub input: Json,
    pub optional_params: Params,
    pub api_base: String,
    pub litellm_params: Params,
    pub headers: Headers,
}

#[derive(Debug, Clone)]
pub struct SignRequest {
    pub headers: Headers,
    pub optional_params: Params,
    pub request_data: Params,
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub model: Option<String>,
    pub stream: Option<bool>,
    pub fake_stream: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponsesUpstream {
    pub status_code: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponsesLogging {
    pub model_call_details: Params,
}

pub const fn api_base_or_required(api_base: Option<&str>) -> Result<&str, ResponsesContractError> {
    let Some(base) = api_base else {
        return Err(ResponsesContractError::ApiBaseRequired);
    };
    Ok(base)
}

#[must_use]
pub fn strip_custom_call_namespace(input: &Json) -> Json {
    let Json::List(items) = input else {
        return input.clone();
    };
    Json::List(items.iter().map(strip_item_namespace).collect())
}

fn strip_item_namespace(item: &Json) -> Json {
    let Json::Object(fields) = item else {
        return item.clone();
    };
    let is_custom_call =
        matches!(fields.get("type"), Some(Json::Text(name)) if name == "custom_tool_call");
    if !is_custom_call {
        return item.clone();
    }
    Json::Object(
        fields
            .iter()
            .filter(|(name, _)| name.as_str() != "namespace")
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

#[must_use]
pub fn normalize_responses_request(data: &Params) -> Params {
    let Some(input) = data.get("input") else {
        return data.clone();
    };
    let mut normalized = data.clone();
    normalized.insert("input".to_owned(), strip_custom_call_namespace(input));
    normalized
}
