pub mod error;

use std::future::Future;

use betterlite_config::sections::xai_identity::Settings;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::api::responses_event_records::records::KnownResponseEvent;
use crate::api::responses_objects::{ResponseApiUsage, ResponsesApiResponse};
use crate::openai_base_config::{Headers, Object};
use crate::response_events::event_decode::DecodedStreamResponse;
use crate::response_events::{ReplyBody, ResponsesReply};
use crate::xai_identity::{KeyOrder, get_api_key};
use crate::xai_tokens::error::XaiOauthError;
use crate::xai_usage::xai_reported_cost_in_usd;
use error::XaiResponsesError;

const DEFAULT_API_BASE: &str = "https://api.x.ai/v1";
const WEB_SEARCH_TYPE: &str = "web_search";
const X_SEARCH_TYPE: &str = "x_search";
const CODE_INTERPRETER_TYPE: &str = "code_interpreter";
const SEARCH_CONTEXT_SIZE: &str = "search_context_size";
const NESTED_FILTERS: &str = "filters";
const X_SEARCH_FIELDS: [&str; 6] = [
    "allowed_x_handles",
    "excluded_x_handles",
    "from_date",
    "to_date",
    "enable_image_understanding",
    "enable_video_understanding",
];

pub trait OauthAccess {
    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, XaiOauthError>> + Send;
}

// LiteLLM's map_openai_params: metadata is always dropped and every tool is rebuilt in xAI's shape
pub fn map_openai_params(params: &mut Object) {
    if params.remove("metadata").is_some() {
        tracing::debug!("XAI Responses API does not support 'metadata' parameter. Dropping it.");
    }
    let Some(tools) = params.get("tools") else {
        return;
    };
    if !truthy(tools) {
        return;
    }
    let transformed = tools_of(tools)
        .iter()
        .map(transform_tool)
        .collect::<Vec<Value>>();
    params.insert(String::from("tools"), Value::Array(transformed));
}

// Python's truthiness, because a non-list tools value is normalized only when it is truthy
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

fn tools_of(value: &Value) -> &[Value] {
    match value {
        Value::Array(items) => items,
        other => std::slice::from_ref(other),
    }
}

fn transform_tool(tool: &Value) -> Value {
    let Value::Object(fields) = tool else {
        return tool.clone();
    };
    match fields.get("type").and_then(Value::as_str) {
        Some(CODE_INTERPRETER_TYPE) => {
            tracing::debug!("XAI: Transforming code_interpreter tool, removing container field");
            let mut tool = Object::new();
            tool.insert(
                String::from("type"),
                Value::String(String::from(CODE_INTERPRETER_TYPE)),
            );
            Value::Object(tool)
        }
        Some(WEB_SEARCH_TYPE) => {
            tracing::debug!("XAI: Transforming web_search tool to XAI format");
            transform_web_search_tool(fields)
        }
        Some(X_SEARCH_TYPE) => {
            tracing::debug!("XAI: Transforming x_search tool to XAI format");
            transform_x_search_tool(fields)
        }
        Some(_) | None => tool.clone(),
    }
}

fn transform_web_search_tool(fields: &Object) -> Value {
    let mut tool = Object::new();
    tool.insert(
        String::from("type"),
        Value::String(String::from(WEB_SEARCH_TYPE)),
    );
    if fields.contains_key(SEARCH_CONTEXT_SIZE) {
        tracing::info!(
            "XAI does not support 'search_context_size' parameter. Removing it from web_search tool."
        );
    }
    // a mapping filters value replaces the flat fields, even when it carries neither domain list
    let domains = match fields.get(NESTED_FILTERS) {
        Some(Value::Object(nested)) => nested,
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_),
        )
        | None => fields,
    };
    let mut filters = Object::new();
    for name in ["allowed_domains", "excluded_domains"] {
        if let Some(value) = domains.get(name) {
            filters.insert(String::from(name), value.clone());
        }
    }
    if !filters.is_empty() {
        tool.insert(String::from(NESTED_FILTERS), Value::Object(filters));
    }
    if let Some(value) = fields.get("enable_image_understanding") {
        tool.insert(String::from("enable_image_understanding"), value.clone());
    }
    Value::Object(tool)
}

fn transform_x_search_tool(fields: &Object) -> Value {
    let mut tool = Object::new();
    tool.insert(
        String::from("type"),
        Value::String(String::from(X_SEARCH_TYPE)),
    );
    for name in X_SEARCH_FIELDS {
        if let Some(value) = fields.get(name) {
            tool.insert(String::from(name), value.clone());
        }
    }
    Value::Object(tool)
}

pub async fn validate_environment<A>(
    headers: Headers,
    api_key: Option<&SecretString>,
    model: &str,
    use_xai_oauth: bool,
    settings: &Settings,
    oauth: &mut A,
) -> Result<Headers, XaiResponsesError>
where
    A: OauthAccess + ?Sized,
{
    let mut resolved = get_api_key(api_key, KeyOrder::GenericBeforeEnvironment, settings);
    if resolved.is_none() && use_xai_oauth {
        resolved = Some(oauth.access_token().await.map_err(|source| {
            XaiResponsesError::Authentication {
                model: model.to_owned(),
                source,
            }
        })?);
    }
    let key = resolved.ok_or(XaiResponsesError::MissingApiKey)?;
    let mut headers = headers;
    headers.insert(
        String::from("Authorization"),
        format!("Bearer {}", key.expose_secret()),
    );
    Ok(headers)
}

// LiteLLM's get_complete_url: OAuth picks its own base only when it is opted into and no key resolved
#[must_use]
pub fn get_complete_url(
    api_base: Option<&str>,
    litellm_api_base: Option<&str>,
    api_key: Option<&SecretString>,
    use_xai_oauth: bool,
    settings: &Settings,
    oauth_base: &str,
) -> String {
    let resolved = get_api_key(api_key, KeyOrder::GenericBeforeEnvironment, settings);
    let base = if use_xai_oauth && resolved.is_none() {
        oauth_base
    } else {
        api_base
            .or(litellm_api_base)
            .or(settings.api_base.as_deref())
            .unwrap_or(DEFAULT_API_BASE)
    };
    format!("{}/responses", base.trim_end_matches('/'))
}

fn restated_cost(usage: &ResponseApiUsage) -> Option<Decimal> {
    let ticks = usage.extra.get("cost_in_usd_ticks")?;
    let value = serde_json::to_value(ticks).ok()?;
    xai_reported_cost_in_usd(&value)
}

pub fn restate_response_usage(response: &mut ResponsesApiResponse) {
    let Some(usage) = &mut response.usage else {
        return;
    };
    let Some(cost) = restated_cost(usage) else {
        return;
    };
    usage.cost = Some(cost);
}

pub fn restate_reply_usage(reply: &mut ResponsesReply) {
    let ReplyBody::Typed(response) = &mut reply.response else {
        return;
    };
    restate_response_usage(response);
}

pub fn restate_event_usage(event: &mut DecodedStreamResponse) {
    let DecodedStreamResponse::Known { record, .. } = event else {
        return;
    };
    if let KnownResponseEvent::ResponseCompleted(fields)
    | KnownResponseEvent::ResponseIncomplete(fields)
    | KnownResponseEvent::ResponseFailed(fields) = record.as_mut()
    {
        restate_response_usage(&mut fields.response);
    }
}

#[must_use]
pub const fn supports_native_websocket() -> bool {
    false
}

mod tests;
