use betterlite_http::url_path::encode_segment;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use serde_json::Number;
use url::Url;

use super::error::ResponseResourceError;
use crate::response_input::{ResponsesInput, transform_responses_request};

pub const DEFAULT_INPUT_ITEMS_LIMIT: i64 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRequest {
    pub url: String,
    pub payload: Object,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceOperation {
    Delete,
    Get,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputItemsOrder {
    Asc,
    Desc,
}

impl InputItemsOrder {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListInputItemsQuery {
    pub after: Option<String>,
    pub before: Option<String>,
    pub include: Option<Vec<String>>,
    pub limit: Option<i64>,
    pub order: Option<InputItemsOrder>,
}

impl Default for ListInputItemsQuery {
    fn default() -> Self {
        Self {
            after: None,
            before: None,
            include: None,
            limit: Some(DEFAULT_INPUT_ITEMS_LIMIT),
            order: Some(InputItemsOrder::Desc),
        }
    }
}

pub struct CompactRequest<'request> {
    pub model: &'request str,
    pub input: &'request ResponsesInput,
    pub options: &'request Object,
    pub provider: ProviderId,
    pub model_info: Option<&'request Object>,
}

pub fn resource_request(
    response_id: &str,
    api_base: &str,
    operation: ResourceOperation,
) -> Result<ResourceRequest, ResponseResourceError> {
    let encoded = encode_segment(response_id, "response_id")
        .map_err(|source| ResponseResourceError::ResponseId { source })?;
    let suffix = match operation {
        ResourceOperation::Delete | ResourceOperation::Get => "",
        ResourceOperation::Cancel => "/cancel",
    };
    let url = format!("{api_base}/{encoded}{suffix}");
    Ok(ResourceRequest {
        url,
        payload: Object::new(),
    })
}

pub fn list_input_items_request(
    response_id: &str,
    api_base: &str,
    query: &ListInputItemsQuery,
) -> Result<ResourceRequest, ResponseResourceError> {
    let encoded = encode_segment(response_id, "response_id")
        .map_err(|source| ResponseResourceError::ResponseId { source })?;
    let url = format!("{api_base}/{encoded}/input_items");
    let mut payload = Object::new();
    if let Some(after) = &query.after {
        payload.insert(String::from("after"), Json::Text(after.clone()));
    }
    if let Some(before) = &query.before {
        payload.insert(String::from("before"), Json::Text(before.clone()));
    }
    if let Some(include) = query.include.as_ref().filter(|values| !values.is_empty()) {
        payload.insert(String::from("include"), Json::Text(include.join(",")));
    }
    if let Some(limit) = query.limit {
        payload.insert(String::from("limit"), Json::Number(Number::from(limit)));
    }
    if let Some(order) = query.order {
        payload.insert(String::from("order"), Json::Text(order.as_str().to_owned()));
    }
    Ok(ResourceRequest { url, payload })
}

pub fn compact_request(
    request: &CompactRequest<'_>,
    api_base: &str,
) -> Result<ResourceRequest, ResponseResourceError> {
    let url = compact_url(api_base)?;
    let payload = transform_responses_request(
        request.model,
        request.input,
        request.options,
        request.provider,
        request.model_info,
    )?;
    Ok(ResourceRequest { url, payload })
}

fn compact_url(api_base: &str) -> Result<String, ResponseResourceError> {
    let mut url =
        Url::parse(api_base).map_err(|source| ResponseResourceError::ApiBase { source })?;
    let path = format!("{}/compact", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url.to_string())
}
