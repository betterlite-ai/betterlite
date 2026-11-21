use betterlite_context::owned_parameter_names::is_litellm_owned_kwarg;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::Number;

use super::super::super::call::{GenericParams, ImageGenerationCall};
use super::super::super::operations::Params;

// litellm.image_generation's `openai_params`, the names filter_out_litellm_params excludes
const OPENAI_PARAMS: [&str; 15] = [
    "user",
    "request_timeout",
    "api_base",
    "api_version",
    "api_key",
    "deployment_id",
    "organization",
    "base_url",
    "default_headers",
    "timeout",
    "max_retries",
    "n",
    "quality",
    "size",
    "style",
];

pub(super) fn passed_params(call: &ImageGenerationCall) -> IndexMap<String, Json> {
    let mut params: IndexMap<String, Json> = call
        .kwargs
        .iter()
        .filter(|(name, _)| {
            !OPENAI_PARAMS.contains(&name.as_str()) && !is_litellm_owned_kwarg(name)
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    // both bind get_optional_params_image_gen's named arguments instead of reaching its **kwargs
    params.shift_remove("drop_params");
    params.shift_remove("additional_drop_params");
    if let Some(n) = call.n {
        params.insert(String::from("n"), Json::Number(Number::from(n)));
    }
    for (name, value) in [
        ("quality", &call.quality),
        ("response_format", &call.response_format),
        ("size", &call.size),
        ("style", &call.style),
        ("user", &call.user),
    ] {
        if let Some(text) = value {
            params.insert(name.to_owned(), Json::Text(text.clone()));
        }
    }
    params
}

// the keys get_litellm_params would carry for this call: the credentials the Azure token provider reads
pub(super) fn litellm_params(generic: &GenericParams) -> Params {
    let mut params = Params::new();
    params.insert(String::from("api_base"), Json::Null);
    let credentials = &generic.credentials;
    for (name, value) in [
        ("tenant_id", &credentials.tenant_id),
        ("client_id", &credentials.client_id),
        ("azure_username", &credentials.azure_username),
        ("azure_scope", &credentials.azure_scope),
    ] {
        if let Some(text) = value {
            params.insert(name.to_owned(), Json::Text(text.clone()));
        }
    }
    for (name, value) in [
        ("client_secret", &credentials.client_secret),
        ("azure_password", &credentials.azure_password),
    ] {
        if let Some(secret) = value {
            params.insert(
                name.to_owned(),
                Json::Text(secret.expose_secret().to_owned()),
            );
        }
    }
    if let Some(token) = &generic.azure_ad_token {
        params.insert(String::from("azure_ad_token"), token.clone());
    }
    params
}

pub(super) fn litellm_literal(call: &ImageGenerationCall) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    params.insert(
        String::from("timeout"),
        Number::from_f64(call.timeout).map_or(Json::Null, Json::Number),
    );
    params.insert(String::from("azure"), Json::Bool(false));
    for name in [
        "litellm_call_id",
        "logger_fn",
        "proxy_server_request",
        "model_info",
    ] {
        params.insert(
            name.to_owned(),
            call.kwargs.get(name).cloned().unwrap_or(Json::Null),
        );
    }
    params.insert(String::from("preset_cache_key"), Json::Null);
    params.insert(
        String::from("stream_response"),
        Json::Object(IndexMap::new()),
    );
    params
}
