use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::Number;

use super::error::ResponsesWsHistoryError;
use super::provider::same_provider;
use super::warmup::warmup_source_params;
use super::{FrameSink, ManagedResponsesWebSocket};
use crate::api::response_synthetic_events::python_value::has_content;
use crate::api::responses_parameters::requested::RESPONSES_OPTIONAL_PARAMS;

const OPTIONAL_PROXY_FIELDS: [&str; 4] = ["tools", "tool_choice", "instructions", "metadata"];

// ResponsesAPIRequestParams.__required_keys__ and __optional_keys__ at the pinned commit
pub(crate) fn response_create_params() -> impl Iterator<Item = &'static str> {
    ["input", "model"]
        .into_iter()
        .chain(RESPONSES_OPTIONAL_PARAMS.split_whitespace())
}

#[must_use]
pub fn build_base_call_kwargs(message: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let source = warmup_source_params(message);
    let mut call_kwargs = IndexMap::new();
    for name in response_create_params() {
        if let Some(value) = source.get(name)
            && !matches!(value, Json::Null)
        {
            call_kwargs.insert(name.to_owned(), value.clone());
        }
    }
    call_kwargs
}

fn proxy_body(
    call_kwargs: &IndexMap<String, Json>,
    proxy_server_request: &IndexMap<String, Json>,
    model: &str,
) -> Result<IndexMap<String, Json>, ResponsesWsHistoryError> {
    let mut body = match proxy_server_request.get("body") {
        Some(Json::Object(fields)) => fields.clone(),
        Some(other) if has_content(other) => {
            return Err(ResponsesWsHistoryError::Shape {
                field: "body",
                expected: "an object",
            });
        }
        _ => IndexMap::new(),
    };
    body.insert(
        "input".to_owned(),
        call_kwargs.get("input").cloned().unwrap_or(Json::Null),
    );
    body.insert(
        "store".to_owned(),
        call_kwargs.get("store").cloned().unwrap_or(Json::Null),
    );
    body.insert("model".to_owned(), Json::Text(model.to_owned()));
    for name in OPTIONAL_PROXY_FIELDS {
        if let Some(value) = call_kwargs.get(name)
            && !matches!(value, Json::Null)
        {
            body.insert(name.to_owned(), value.clone());
        }
    }
    Ok(body)
}

pub fn update_proxy_request(
    call_kwargs: &mut IndexMap<String, Json>,
    model: &str,
) -> Result<(), ResponsesWsHistoryError> {
    let metadata = match call_kwargs.get("litellm_metadata") {
        Some(Json::Object(fields)) => fields.clone(),
        None | Some(Json::Null) => IndexMap::new(),
        Some(_) => {
            return Err(ResponsesWsHistoryError::Shape {
                field: "litellm_metadata",
                expected: "an object",
            });
        }
    };
    let mut proxy = match metadata.get("proxy_server_request") {
        Some(Json::Object(fields)) => fields.clone(),
        Some(other) if has_content(other) => return Ok(()),
        _ => IndexMap::new(),
    };
    let body = proxy_body(call_kwargs, &proxy, model)?;
    proxy.insert("body".to_owned(), Json::Object(body));
    let stored = Json::Object(proxy);
    let mut metadata_fields = metadata;
    metadata_fields.insert("proxy_server_request".to_owned(), stored.clone());
    call_kwargs.insert("litellm_metadata".to_owned(), Json::Object(metadata_fields));
    call_kwargs.insert("proxy_server_request".to_owned(), stored);
    Ok(())
}

pub fn inject_credentials<Sink: FrameSink>(
    handler: &ManagedResponsesWebSocket<Sink>,
    call_kwargs: &mut IndexMap<String, Json>,
    model: Option<&str>,
) -> Result<(), ResponsesWsHistoryError> {
    if let Some(api_key) = &handler.api_key {
        let key = Json::Text(api_key.expose_secret().to_owned());
        call_kwargs.insert("api_key".to_owned(), key);
    }
    if let Some(api_base) = &handler.api_base {
        call_kwargs.insert("api_base".to_owned(), Json::Text(api_base.clone()));
    }
    if let Some(timeout) = handler.timeout {
        let number = Number::from_f64(timeout).ok_or(ResponsesWsHistoryError::Shape {
            field: "timeout",
            expected: "a finite number",
        })?;
        call_kwargs.insert("timeout".to_owned(), Json::Number(number));
    }
    if let Some(provider) = &handler.custom_llm_provider
        && same_provider(handler, model)
    {
        call_kwargs.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(provider.clone()),
        );
    }
    if !handler.litellm_metadata.is_empty() {
        let fields = Json::Object(handler.litellm_metadata.clone());
        call_kwargs.insert("litellm_metadata".to_owned(), fields);
    }
    Ok(())
}
