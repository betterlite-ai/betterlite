use std::collections::BTreeMap;

use betterlite_azure::azure_urls::{V1_API_VERSIONS, get_base_azure_url};
use betterlite_azure::chat_call::request::{
    self, EXTRA_HEADERS_KEY, JSON_MODE_KEY, Retries, STREAM_KEY, build_data,
};
use betterlite_azure::completion::request::{Retries as TextRetries, build_data as text_data};
use betterlite_config::sections::azure_endpoints::DEFAULT_MAX_RETRIES;
use betterlite_models::decisions::Extra;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::azure::text::azure_text_pt;
use betterlite_prompts::message_content::Message;
use secrecy::ExposeSecret as _;

use super::error::AzureProviderError;
use super::resolve::{Resolved, convert};

// The request as it will be written: one place builds it for the route and for the typed dispatcher.
pub struct Wire {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Object,
    pub json_mode: bool,
}

fn v1_version(version: Option<&str>) -> bool {
    version.is_some_and(|text| V1_API_VERSIONS.contains(&text))
}

// the SDK's auth header: a key is api-key, or Bearer on the v1 surface; a token is always Bearer
fn auth_headers(resolved: &Resolved, version: Option<&str>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::from([
        (String::from("accept"), String::from("application/json")),
        (
            String::from("content-type"),
            String::from("application/json"),
        ),
    ]);
    let key = resolved
        .api_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty());
    // the SDK checks its token first, then the key: AzureOpenAI sends Bearer when both are set
    if let Some(token) = resolved
        .auth_token
        .as_ref()
        .filter(|token| !token.expose_secret().is_empty())
    {
        headers.insert(
            String::from("authorization"),
            format!("Bearer {}", token.expose_secret()),
        );
    } else if let Some(key) = key {
        if v1_version(version) {
            headers.insert(
                String::from("authorization"),
                format!("Bearer {}", key.expose_secret()),
            );
        } else {
            headers.insert(String::from("api-key"), key.expose_secret().to_owned());
        }
    }
    headers
}

fn url_for(
    api_base: Option<&str>,
    api_version: Option<&str>,
    model: &str,
    route: &str,
) -> Result<String, AzureProviderError> {
    let Some(base) = api_base else {
        return get_base_azure_url(None, api_version, None, route)
            .map_err(|source| AzureProviderError::Url { source });
    };
    let composed = if base.contains("/openai/deployments") || base.contains(route) {
        get_base_azure_url(Some(base), api_version, None, route)
    } else if v1_version(api_version) {
        let v1 = format!("{}/openai/v1", base.trim_end_matches('/'));
        get_base_azure_url(Some(&v1), None, None, route)
    } else {
        let full = format!("/openai/deployments/{model}{route}");
        get_base_azure_url(Some(base), api_version, None, &full)
    };
    composed.map_err(|source| AzureProviderError::Url { source })
}

// if headers: optional_params["extra_headers"] = headers
fn insert_extra(optional: &mut Object, headers: &BTreeMap<String, String>) {
    request::inject_headers(optional, &SecretHeaders(headers.clone()));
}

fn take_extra(optional: &mut Object) -> Result<BTreeMap<String, String>, AzureProviderError> {
    let fields = match optional.shift_remove(EXTRA_HEADERS_KEY) {
        None | Some(Json::Null) => return Ok(BTreeMap::new()),
        Some(Json::Object(fields)) => fields,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(AzureProviderError::HeadersNotObject);
        }
    };
    let mut headers = BTreeMap::new();
    for (name, value) in fields {
        let Json::Text(text) = value else {
            return Err(AzureProviderError::HeaderValueType { name });
        };
        headers.insert(name, text);
    }
    Ok(headers)
}

// LiteLLM folds kwargs.headers into optional_params["extra_headers"] after kwargs.extra_headers.
fn chat_extra(
    resolved: &Resolved,
    optional: &mut Object,
) -> Result<BTreeMap<String, String>, AzureProviderError> {
    // completion() merges kwargs.headers with kwargs.extra_headers before the handler sees them
    let mut merged = resolved.headers.clone().unwrap_or_default();
    if let Some(extra) = &resolved.extra_headers {
        merged.extend(extra.clone());
    }
    let sent = if merged.is_empty() {
        resolved.globals_headers.clone()
    } else {
        Some(merged)
    };
    if let Some(headers) = &sent {
        insert_extra(optional, headers);
    }
    take_extra(optional)
}

fn retries(optional: &mut Object, max_retries: Option<&Json>) -> Result<(), AzureProviderError> {
    if let Some(value) = max_retries.filter(|value| **value != Json::Null) {
        optional.insert(String::from("max_retries"), value.clone());
    }
    if !Retries::pop(optional, DEFAULT_MAX_RETRIES).is_int() {
        return Err(AzureProviderError::MaxRetriesType);
    }
    Ok(())
}

fn with_auth(
    resolved: &Resolved,
    extra: BTreeMap<String, String>,
    api_version: Option<&str>,
) -> BTreeMap<String, String> {
    let mut headers = auth_headers(resolved, api_version);
    headers.extend(extra);
    headers
}

pub fn chat(
    resolved: &Resolved,
    model: &str,
    messages: &[Message],
    mut optional: Object,
    stream: bool,
    base_model: Option<&str>,
    openai_system_messages_first: bool,
) -> Result<Wire, AzureProviderError> {
    let extra = chat_extra(resolved, &mut optional)?;
    retries(&mut optional, resolved.max_retries.as_ref())?;
    let json_mode = matches!(optional.shift_remove(JSON_MODE_KEY), Some(Json::Bool(true)));
    let mut body = build_data(
        model,
        messages,
        &optional,
        base_model,
        openai_system_messages_first,
    )?;
    // the SDK merges extra_body over the body it was handed
    for (name, value) in &resolved.extra_body {
        body.insert(name.clone(), value.clone());
    }
    if stream {
        body.insert(String::from(STREAM_KEY), Json::Bool(true));
    }
    let api_version = resolved.api_version.as_deref();
    let url = url_for(
        resolved.api_base.as_deref(),
        api_version,
        model,
        "/chat/completions",
    )?;
    Ok(Wire {
        url,
        headers: with_auth(resolved, extra, api_version),
        body,
        json_mode,
    })
}

// Text never folds kwargs.headers: only kwargs.extra_headers reach the request.
pub fn text(
    resolved: &Resolved,
    model: &str,
    messages: &[Message],
    mut optional: Object,
) -> Result<Wire, AzureProviderError> {
    // a non-None extra_headers overwrites whatever the body carried, even when it is empty
    if let Some(extra) = &resolved.extra_headers {
        insert_extra(&mut optional, extra);
    }
    let extra = take_extra(&mut optional)?;
    let mut optional: Extra = convert(&optional)?;
    TextRetries::pop(&mut optional);
    let prompt = azure_text_pt(messages).map_err(|error| AzureProviderError::Rejected {
        status: 500,
        message: error.to_string(),
    })?;
    let cloudflare = resolved.api_base.as_deref().is_some_and(|base| {
        base.contains(betterlite_azure::completion::request::CLOUDFLARE_MARKER)
    });
    let body: Object = convert(&text_data(Some(model), &prompt, &optional, cloudflare))?;
    let api_base = resolved
        .api_base
        .as_deref()
        .ok_or(AzureProviderError::MissingTextApiBase)?;
    // _complete_azure_text passes None through, and the SDK's client then applies its own default
    let version = resolved
        .api_version
        .clone()
        .unwrap_or_else(|| resolved.default_api_version.clone());
    let api_version = Some(version.as_str());
    let url = url_for(Some(api_base), api_version, model, "/completions")?;
    Ok(Wire {
        url,
        headers: with_auth(resolved, extra, api_version),
        body,
        json_mode: false,
    })
}

// The call kwargs LiteLLM reads before the provider body; none of them reach the provider
const EMBEDDING_CONTROL_KEYS: [&str; 9] = [
    "api_base",
    "api_key",
    "api_version",
    "timeout",
    "max_retries",
    "headers",
    "extra_headers",
    "extra_body",
    "metadata",
];

pub fn embedding(
    resolved: &Resolved,
    model: &str,
    request: &EmbeddingRequest,
) -> Result<Wire, AzureProviderError> {
    let mut body: Object = convert(request)?;
    body.insert(String::from("model"), Json::Text(model.to_owned()));
    body.shift_remove("azure_ad_token");
    for name in EMBEDDING_CONTROL_KEYS {
        body.shift_remove(name);
    }
    // the SDK's own client default, applied before LiteLLM ever sees the request
    body.entry(String::from("encoding_format"))
        .or_insert_with(|| Json::Text(String::from("base64")));
    // main.py merges kwargs.headers and kwargs.extra_headers, extra_headers winning, and sends the map only when non-empty
    let mut supplied = resolved.headers.clone().unwrap_or_default();
    if let Some(extra) = &resolved.extra_headers {
        supplied.extend(extra.clone());
    }
    if !supplied.is_empty() {
        insert_extra(&mut body, &supplied);
    }
    let extra = take_extra(&mut body)?;
    for (name, value) in &resolved.extra_body {
        body.insert(name.clone(), value.clone());
    }
    let api_base = resolved
        .api_base
        .as_deref()
        .ok_or(AzureProviderError::MissingEmbeddingApiBase)?;
    let api_version = resolved.api_version.as_deref();
    let url = url_for(Some(api_base), api_version, model, "/embeddings")?;
    Ok(Wire {
        url,
        headers: with_auth(resolved, extra, api_version),
        body,
        json_mode: false,
    })
}
