use std::collections::BTreeMap;

use axum::http::header::{HeaderMap, HeaderName, HeaderValue};
use betterlite_auth::dev_auth_mode::{AuthModeFlags, AuthModeSwitch};
use betterlite_auth::passthrough::caller_headers::caller_filter::{
    FilterSettings, caller_headers_without_secrets,
};
use betterlite_auth::passthrough::caller_headers::credential_value::{
    get_litellm_virtual_key, normalize_credential_value,
};
use betterlite_auth::passthrough::caller_headers::error::CallerHeaderError;
use betterlite_auth::passthrough::caller_headers::upstream::{
    anthropic_upstream_headers, bedrock_runtime_headers, vertex_credentialless_headers,
};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

type HeaderPair = (String, String);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    virtual_key: Option<VirtualKeyCall>,
    #[serde(default)]
    normalize: Option<NormalizeCall>,
    #[serde(default)]
    filter: Option<FilterCall>,
    #[serde(default)]
    vertex: Option<ProviderCall>,
    #[serde(default)]
    anthropic: Option<ProviderCall>,
    #[serde(default)]
    bedrock: Option<ProviderCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VirtualKeyCall {
    headers: Vec<HeaderPair>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NormalizeCall {
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FilterCall {
    headers: Vec<HeaderPair>,
    never_forwarded: Vec<String>,
    caller: CallerInput,
    settings: SettingsInput,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderCall {
    headers: Vec<HeaderPair>,
    caller: CallerInput,
    settings: SettingsInput,
    #[serde(default)]
    proxy_auth_header: Option<Vec<HeaderPair>>,
    #[serde(default)]
    signed_headers: Vec<HeaderPair>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallerInput {
    stored_api_key: Option<String>,
    jwt_claims: Option<Payload>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsInput {
    master_key: Option<String>,
    custom_auth_configured: bool,
    auth_mode: Vec<AuthSwitch>,
    litellm_key_header_name: Option<String>,
    endpoint_headers: Vec<JsonObject>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AuthSwitch {
    #[serde(rename = "enable_jwt_auth")]
    Jwt,
    #[serde(rename = "enable_oauth2_auth")]
    Oauth2,
    #[serde(rename = "enable_oauth2_proxy_auth")]
    Oauth2Proxy,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        if let Some(call) = &input.virtual_key {
            return virtual_key_answer(call);
        }
        if let Some(call) = &input.normalize {
            return Ok(normalize_answer(call));
        }
        if let Some(call) = &input.filter {
            return filter_answer(call);
        }
        if let Some(call) = &input.vertex {
            return provider_answer(call, ProviderRoute::Vertex);
        }
        if let Some(call) = &input.anthropic {
            return provider_answer(call, ProviderRoute::Anthropic);
        }
        if let Some(call) = &input.bedrock {
            return provider_answer(call, ProviderRoute::Bedrock);
        }
        Err(missing_operation())
    }
}

fn virtual_key_answer(call: &VirtualKeyCall) -> HelperResult {
    let headers = header_map(&call.headers)?;
    let key = get_litellm_virtual_key(&headers);
    Ok(json!({"value": key.expose_secret()}))
}

fn normalize_answer(call: &NormalizeCall) -> Value {
    let value = SecretString::from(call.value.as_str());
    let normalized = normalize_credential_value(&value);
    json!({"value": normalized.expose_secret()})
}

fn filter_answer(call: &FilterCall) -> HelperResult {
    let master_key = call.settings.master_key.as_deref().map(SecretString::from);
    let endpoints: Vec<&JsonObject> = call.settings.endpoint_headers.iter().collect();
    let settings = filter_settings(&call.settings, master_key.as_ref(), &endpoints);
    let never_forwarded: Vec<&str> = call.never_forwarded.iter().map(String::as_str).collect();
    let headers = header_map(&call.headers)?;
    let caller = caller_auth(&call.caller)?;
    let forwarded = caller_headers_without_secrets(&headers, &caller, &never_forwarded, &settings);
    Ok(json!({"headers": header_object(&forwarded)}))
}

#[derive(Debug, Clone, Copy)]
enum ProviderRoute {
    Vertex,
    Anthropic,
    Bedrock,
}

fn provider_answer(call: &ProviderCall, route: ProviderRoute) -> HelperResult {
    let master_key = call.settings.master_key.as_deref().map(SecretString::from);
    let endpoints: Vec<&JsonObject> = call.settings.endpoint_headers.iter().collect();
    let settings = filter_settings(&call.settings, master_key.as_ref(), &endpoints);
    let headers = header_map(&call.headers)?;
    let caller = caller_auth(&call.caller)?;
    match route {
        ProviderRoute::Vertex => {
            headers_or_failure(vertex_credentialless_headers(&headers, &caller, &settings))
        }
        ProviderRoute::Anthropic => {
            let proxy = call.proxy_auth_header.as_deref().map(secret_map);
            headers_or_failure(anthropic_upstream_headers(
                &headers,
                &caller,
                proxy.as_ref(),
                &settings,
            ))
        }
        ProviderRoute::Bedrock => {
            let signed = secret_map(&call.signed_headers);
            let forwarded = bedrock_runtime_headers(&headers, &caller, &signed, &settings);
            Ok(json!({"headers": header_object(&forwarded)}))
        }
    }
}

fn filter_settings<'settings>(
    input: &'settings SettingsInput,
    master_key: Option<&'settings SecretString>,
    endpoint_headers: &'settings [&'settings JsonObject],
) -> FilterSettings<'settings> {
    FilterSettings {
        master_key,
        auth_mode: AuthModeFlags {
            enable_jwt_auth: AuthModeSwitch(input.auth_mode.contains(&AuthSwitch::Jwt)),
            enable_oauth2_auth: AuthModeSwitch(input.auth_mode.contains(&AuthSwitch::Oauth2)),
            enable_oauth2_proxy_auth: AuthModeSwitch(
                input.auth_mode.contains(&AuthSwitch::Oauth2Proxy),
            ),
        },
        custom_auth_configured: input.custom_auth_configured,
        litellm_key_header_name: input.litellm_key_header_name.as_deref(),
        endpoint_headers,
    }
}

fn caller_auth(call: &CallerInput) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    if let Some(stored) = &call.stored_api_key {
        fields.insert("api_key".to_owned(), JsonData::String(stored.clone()));
    }
    if let Some(claims) = &call.jwt_claims {
        fields.insert("jwt_claims".to_owned(), JsonData::Object(claims.0.clone()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()
}

fn header_map(pairs: &[HeaderPair]) -> Result<HeaderMap, Box<HelperError>> {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        let header_name = HeaderName::from_bytes(name.as_bytes()).or_value_error()?;
        let header_value = HeaderValue::from_bytes(value.as_bytes()).or_value_error()?;
        map.append(header_name, header_value);
    }
    Ok(map)
}

fn secret_map(pairs: &[HeaderPair]) -> IndexMap<String, SecretString> {
    pairs
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect()
}

fn header_object(forwarded: &IndexMap<String, SecretString>) -> Value {
    let mut object = BTreeMap::new();
    for (name, value) in forwarded {
        object.insert(name.as_str(), value.expose_secret());
    }
    json!(object)
}

fn headers_or_failure(
    result: Result<IndexMap<String, SecretString>, CallerHeaderError>,
) -> HelperResult {
    match result {
        Ok(forwarded) => Ok(json!({"headers": header_object(&forwarded)})),
        Err(error) => Err(caller_failure(&error)),
    }
}

fn caller_failure(error: &CallerHeaderError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "HTTPException".to_owned(),
        status: Some(error.status().as_u16()),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn missing_operation() -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: Some(None),
        param: None,
        code: None,
        message: "the input names no helper operation".to_owned(),
    })
}
