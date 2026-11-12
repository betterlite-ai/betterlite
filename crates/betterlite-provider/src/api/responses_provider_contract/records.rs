use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde::Serialize;

use super::error::ResponsesContractError;

pub type Params = BTreeMap<String, Json>;
pub type Headers = BTreeMap<String, String>;
pub type PreparedRequest = (String, Params);

const DEFAULT_INPUT_ITEMS_LIMIT: i64 = 20;

// The closed set of operations BaseResponsesAPIConfig declares; a new member changes the trait for every provider
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponsesOperation {
    Create,
    Stream,
    Get,
    Delete,
    InputItems,
    Cancel,
    Compact,
}

impl ResponsesOperation {
    pub const ALL: [Self; 7] = [
        Self::Create,
        Self::Stream,
        Self::Get,
        Self::Delete,
        Self::InputItems,
        Self::Cancel,
        Self::Compact,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InputItemsOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct InputItemsQuery<'request> {
    pub after: Option<&'request str>,
    pub before: Option<&'request str>,
    pub include: Option<&'request [String]>,
    pub limit: i64,
    pub order: InputItemsOrder,
}

impl Default for InputItemsQuery<'_> {
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

pub struct CreateRequest<'request> {
    pub model: &'request str,
    pub input: &'request Json,
    pub optional_params: &'request Params,
    pub litellm_params: &'request Params,
    pub headers: &'request Headers,
}

pub struct IdRequest<'request> {
    pub response_id: &'request str,
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
    pub headers: &'request Headers,
}

pub struct InputItemsRequest<'request> {
    pub response_id: &'request str,
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
    pub headers: &'request Headers,
    pub query: InputItemsQuery<'request>,
}

pub struct CompactRequest<'request> {
    pub model: &'request str,
    pub input: &'request Json,
    pub optional_params: &'request Params,
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
    pub headers: &'request Headers,
}

pub struct SignRequest<'request> {
    pub headers: &'request Headers,
    pub optional_params: &'request Params,
    pub request_data: &'request Params,
    pub api_base: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub model: Option<&'request str>,
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
