use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::Number;

use super::error::ResponsesWsHistoryError;
use super::values::is_truthy;
use super::warmup::warmup_source_params;
use super::{FrameSink, ManagedResponsesWebSocket};

// ResponsesAPIRequestParams.__required_keys__ | __optional_keys__ at the pinned commit
pub const RESPONSE_CREATE_PARAMS: [&str; 30] = [
    "input",
    "model",
    "include",
    "instructions",
    "max_output_tokens",
    "metadata",
    "parallel_tool_calls",
    "previous_response_id",
    "reasoning",
    "store",
    "background",
    "stream",
    "temperature",
    "text",
    "tool_choice",
    "tools",
    "top_p",
    "truncation",
    "user",
    "service_tier",
    "safety_identifier",
    "prompt",
    "max_tool_calls",
    "prompt_cache_key",
    "prompt_cache_retention",
    "prompt_cache_options",
    "stream_options",
    "top_logprobs",
    "partial_images",
    "context_management",
];

const OPTIONAL_PROXY_FIELDS: [&str; 4] = ["tools", "tool_choice", "instructions", "metadata"];

// _build_base_call_kwargs: only accepted names travel on, and an explicit null is dropped
#[must_use]
pub fn build_base_call_kwargs(message: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let source = warmup_source_params(message);
    let mut call_kwargs = IndexMap::new();
    for name in RESPONSE_CREATE_PARAMS {
        if let Some(value) = source.get(name)
            && !matches!(value, Json::Null)
        {
            call_kwargs.insert(name.to_owned(), value.clone());
        }
    }
    call_kwargs
}

// what an existing proxy_server_request leaves to do: extend it, start fresh, or leave the call alone
enum ProxyRequest {
    Ignored,
    Absent,
    Present(IndexMap<String, Json>),
}

fn metadata_object(
    call_kwargs: &IndexMap<String, Json>,
) -> Result<Option<IndexMap<String, Json>>, ResponsesWsHistoryError> {
    match call_kwargs.get("litellm_metadata") {
        Some(Json::Object(fields)) => Ok(Some(fields.clone())),
        None | Some(Json::Null) => Ok(None),
        Some(_) => Err(ResponsesWsHistoryError::Shape {
            field: "litellm_metadata",
            expected: "an object",
        }),
    }
}

fn existing_proxy_request(metadata: Option<&IndexMap<String, Json>>) -> ProxyRequest {
    match metadata.and_then(|fields| fields.get("proxy_server_request")) {
        Some(Json::Object(fields)) if !fields.is_empty() => ProxyRequest::Present(fields.clone()),
        Some(other) if is_truthy(other) => ProxyRequest::Ignored,
        _ => ProxyRequest::Absent,
    }
}

fn proxy_body(
    proxy: Option<&IndexMap<String, Json>>,
) -> Result<IndexMap<String, Json>, ResponsesWsHistoryError> {
    match proxy.and_then(|fields| fields.get("body")) {
        Some(Json::Object(fields)) if !fields.is_empty() => Ok(fields.clone()),
        Some(other) if is_truthy(other) => Err(ResponsesWsHistoryError::Shape {
            field: "body",
            expected: "an object",
        }),
        _ => Ok(IndexMap::new()),
    }
}

// _update_proxy_request: the spend log body mirrors the call, stored in both documented places
pub fn update_proxy_request(
    call_kwargs: &mut IndexMap<String, Json>,
    model: &str,
) -> Result<(), ResponsesWsHistoryError> {
    let metadata = metadata_object(call_kwargs)?;
    let proxy = match existing_proxy_request(metadata.as_ref()) {
        ProxyRequest::Ignored => return Ok(()),
        ProxyRequest::Absent => None,
        ProxyRequest::Present(fields) => Some(fields),
    };
    let mut body = proxy_body(proxy.as_ref())?;
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
    let mut request = proxy.unwrap_or_else(IndexMap::new);
    request.insert("body".to_owned(), Json::Object(body));
    let stored = Json::Object(request);
    let mut metadata_fields = metadata.unwrap_or_else(IndexMap::new);
    metadata_fields.insert("proxy_server_request".to_owned(), stored.clone());
    call_kwargs.insert("litellm_metadata".to_owned(), Json::Object(metadata_fields));
    call_kwargs.insert("proxy_server_request".to_owned(), stored);
    Ok(())
}

impl<Sink: FrameSink> ManagedResponsesWebSocket<Sink> {
    // _inject_credentials: the connection's key, base and timeout always, its provider only on a matching model
    pub fn inject_credentials(
        &self,
        call_kwargs: &mut IndexMap<String, Json>,
        model: Option<&str>,
    ) -> Result<(), ResponsesWsHistoryError> {
        if let Some(api_key) = &self.api_key {
            call_kwargs.insert(
                "api_key".to_owned(),
                Json::Text(api_key.expose_secret().to_owned()),
            );
        }
        if let Some(api_base) = &self.api_base {
            call_kwargs.insert("api_base".to_owned(), Json::Text(api_base.clone()));
        }
        if let Some(timeout) = self.timeout {
            let number = Number::from_f64(timeout).ok_or(ResponsesWsHistoryError::Shape {
                field: "timeout",
                expected: "a finite number",
            })?;
            call_kwargs.insert("timeout".to_owned(), Json::Number(number));
        }
        if let Some(provider) = &self.custom_llm_provider
            && self.same_provider(model)
        {
            call_kwargs.insert(
                "custom_llm_provider".to_owned(),
                Json::Text(provider.clone()),
            );
        }
        if !self.litellm_metadata.is_empty() {
            call_kwargs.insert(
                "litellm_metadata".to_owned(),
                Json::Object(self.litellm_metadata.clone()),
            );
        }
        Ok(())
    }
}
