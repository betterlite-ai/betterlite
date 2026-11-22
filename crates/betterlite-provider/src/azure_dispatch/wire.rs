use super::EMBEDDING_KEYS;
use super::call::{ChatCall, TextCall};
use super::error::AzureProviderError;
use super::resolve::{Resolved, convert};
use betterlite_azure::azure_urls::{V1_API_VERSIONS, get_base_azure_url};
use betterlite_azure::chat_call::request::{
    self, EXTRA_HEADERS_KEY, JSON_MODE_KEY, Retries, STREAM_KEY, build_data,
};
use betterlite_azure::completion::request::{
    CLOUDFLARE_MARKER, Retries as TextRetries, build_data as text_data,
};
use betterlite_config::sections::azure_endpoints::DEFAULT_MAX_RETRIES;
use betterlite_models::decisions::Extra;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::azure::text::azure_text_pt;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::BTreeMap;
pub(crate) struct Wire {
    pub(crate) url: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Object,
    pub(crate) json_mode: bool,
}
fn present(secret: &Option<SecretString>) -> Option<&str> {
    secret
        .as_ref()
        .map(SecretString::expose_secret)
        .filter(|text| !text.is_empty())
}
// the auth header: the SDK's Azure client prefers its token, while the v1 OpenAI client prefers its key
fn auth_headers(resolved: &Resolved, version: Option<&str>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::from([
        (String::from("accept"), String::from("application/json")),
        (
            String::from("content-type"),
            String::from("application/json"),
        ),
    ]);
    let key = present(&resolved.api_key);
    let token = present(&resolved.auth_token);
    if version.is_some_and(|text| V1_API_VERSIONS.contains(&text)) {
        if let Some(api_key) = key.or(token) {
            headers.insert(String::from("authorization"), format!("Bearer {api_key}"));
        }
    } else if let Some(token) = token {
        headers.insert(String::from("authorization"), format!("Bearer {token}"));
    } else if let Some(api_key) = key {
        headers.insert(String::from("api-key"), api_key.to_owned());
    }
    headers
}
// main.py merges kwargs.headers with kwargs.extra_headers, extra_headers winning, and sends the map only when non-empty
fn merged_headers(resolved: &Resolved) -> Option<BTreeMap<String, String>> {
    let mut merged = resolved.headers.clone().unwrap_or_default();
    if let Some(extra) = &resolved.extra_headers {
        merged.extend(extra.clone());
    }
    (!merged.is_empty()).then_some(merged)
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
    } else if api_version.is_some_and(|text| V1_API_VERSIONS.contains(&text)) {
        let v1 = format!("{}/openai/v1", base.trim_end_matches('/'));
        get_base_azure_url(Some(&v1), None, None, route)
    } else {
        let full = format!("/openai/deployments/{model}{route}");
        get_base_azure_url(Some(base), api_version, None, &full)
    };
    composed.map_err(|source| AzureProviderError::Url { source })
}
fn take_extra(optional: &mut Object) -> Result<BTreeMap<String, String>, AzureProviderError> {
    let fields = match optional.shift_remove(EXTRA_HEADERS_KEY) {
        None | Some(Json::Null) => return Ok(BTreeMap::new()),
        Some(Json::Object(fields)) => fields,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(AzureProviderError::HeadersNotObject);
        }
    };
    fields
        .into_iter()
        .map(|(name, value)| match value {
            Json::Text(text) => Ok((name, text)),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                Err(AzureProviderError::HeaderValueType { name })
            }
        })
        .collect()
}
fn chat_extra(
    resolved: &Resolved,
    optional: &mut Object,
) -> Result<BTreeMap<String, String>, AzureProviderError> {
    let sent = merged_headers(resolved).or_else(|| resolved.globals_headers.clone());
    if let Some(headers) = &sent {
        request::inject_headers(optional, &SecretHeaders(headers.clone()));
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
pub(crate) fn chat(
    resolved: &Resolved,
    call: &ChatCall,
    mut optional: Object,
    openai_system_messages_first: bool,
) -> Result<Wire, AzureProviderError> {
    let extra = chat_extra(resolved, &mut optional)?;
    retries(&mut optional, resolved.max_retries.as_ref())?;
    let json_mode = matches!(optional.shift_remove(JSON_MODE_KEY), Some(Json::Bool(true)));
    let base_model = call.credentials.base_model.as_deref();
    let mut body = build_data(
        call.model.as_str(),
        &call.messages,
        &optional,
        base_model,
        openai_system_messages_first,
    )?;
    // the SDK merges extra_body over the body it was handed
    for (name, value) in &resolved.extra_body {
        body.insert(name.clone(), value.clone());
    }
    if call.stream {
        body.insert(String::from(STREAM_KEY), Json::Bool(true));
    }
    let api_version = resolved.api_version.as_deref();
    let url = url_for(
        resolved.api_base.as_deref(),
        api_version,
        call.model.as_str(),
        "/chat/completions",
    )?;
    let mut headers = auth_headers(resolved, api_version);
    headers.extend(extra);
    Ok(Wire {
        url,
        headers,
        body,
        json_mode,
    })
}
// Text never folds kwargs.headers: only kwargs.extra_headers reach the request.
pub(crate) fn text(
    resolved: &Resolved,
    call: &TextCall,
    mut optional: Object,
) -> Result<Wire, AzureProviderError> {
    // a non-None extra_headers overwrites whatever the body carried, even when it is empty
    if let Some(extra) = &resolved.extra_headers {
        request::inject_headers(&mut optional, &SecretHeaders(extra.clone()));
    }
    let extra = take_extra(&mut optional)?;
    let mut body_options: Extra = convert(&optional)?;
    // kwargs.max_retries is popped before the body is written; text never inserts it back
    let _retries = TextRetries::pop(&mut body_options);
    let prompt = azure_text_pt(&call.messages).map_err(|error| AzureProviderError::Rejected {
        status: 500,
        message: error.to_string(),
    })?;
    let cloudflare = resolved
        .api_base
        .as_deref()
        .is_some_and(|base| base.contains(CLOUDFLARE_MARKER));
    let body: Object = convert(&text_data(
        Some(call.model.as_str()),
        &prompt,
        &body_options,
        cloudflare,
    ))?;
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
    let url = url_for(
        Some(api_base),
        api_version,
        call.model.as_str(),
        "/completions",
    )?;
    let mut headers = auth_headers(resolved, api_version);
    headers.extend(extra);
    Ok(Wire {
        url,
        headers,
        body,
        json_mode: false,
    })
}
pub(crate) fn embedding(
    resolved: &Resolved,
    model: &str,
    request: &EmbeddingRequest,
) -> Result<Wire, AzureProviderError> {
    let mut body: Object = convert(request)?;
    body.insert(String::from("model"), Json::Text(model.to_owned()));
    for name in EMBEDDING_KEYS.split(' ') {
        body.shift_remove(name);
    }
    // the SDK's own client default, applied before LiteLLM ever sees the request
    body.entry(String::from("encoding_format"))
        .or_insert_with(|| Json::Text(String::from("base64")));
    if let Some(headers) = merged_headers(resolved) {
        request::inject_headers(&mut body, &SecretHeaders(headers));
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
    let mut headers = auth_headers(resolved, api_version);
    headers.extend(extra);
    Ok(Wire {
        url,
        headers,
        body,
        json_mode: false,
    })
}
