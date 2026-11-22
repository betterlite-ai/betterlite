use betterlite_azure::endpoints::projection::CallableProvider;
use betterlite_config::sections::azure_endpoints::AZURE_DEFAULT_API_VERSION;
use betterlite_context::chat_config_selection::azure::is_o_series_model;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::call::{Composition, Credentials};
use super::error::AzureProviderError;

pub const DEFAULT_API_TYPE: &str = "azure";

pub struct Resolved {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_type: String,
    // the token the SDK authenticates with: only the call's own azure_ad_token parameter reaches it
    pub auth_token: Option<SecretString>,
    // main.py's resolved token: extra_body.azure_ad_token, then the parameter, then AZURE_AD_TOKEN
    pub azure_ad_token: Option<SecretString>,
    pub azure_ad_token_provider: Option<CallableProvider>,
    // the call's own headers and extra headers, and litellm.headers, each kept for its own merge
    pub headers: Option<Headers>,
    pub extra_headers: Option<Headers>,
    pub globals_headers: Option<Headers>,
    pub max_retries: Option<Json>,
    pub extra_body: Object,
    pub default_api_version: String,
    pub o_series: bool,
}

// The one JSON conversion the two Json types need: a settings object or a body crosses as its own text.
pub fn convert<A, B>(value: &A) -> Result<B, AzureProviderError>
where
    A: Serialize + ?Sized,
    B: DeserializeOwned,
{
    let text =
        serde_json::to_string(value).map_err(|source| AzureProviderError::Encode { source })?;
    serde_json::from_str(&text).map_err(|source| AzureProviderError::Encode { source })
}

fn truthy_text(value: Option<&str>) -> Option<String> {
    value.filter(|text| !text.is_empty()).map(str::to_owned)
}

fn truthy_secret(value: Option<&SecretString>) -> Option<SecretString> {
    value
        .filter(|secret| !secret.expose_secret().is_empty())
        .cloned()
}

// api_base: truthy explicit, litellm.api_base, then AZURE_API_BASE as it stands (an empty value is still set)
fn ranked_base(composition: &Composition, credentials: &Credentials) -> Option<String> {
    truthy_text(credentials.api_base.as_deref())
        .or_else(|| truthy_text(composition.globals.api_base.as_deref()))
        .or_else(|| composition.lookup("AZURE_API_BASE").map(str::to_owned))
}

// api_version: truthy explicit, litellm.api_version, then a non-empty AZURE_API_VERSION
fn ranked_version(composition: &Composition, credentials: &Credentials) -> Option<String> {
    truthy_text(credentials.api_version.as_deref())
        .or_else(|| truthy_text(composition.globals.api_version.as_deref()))
        .or_else(|| truthy_text(composition.lookup("AZURE_API_VERSION")))
}

fn default_version(composition: &Composition) -> String {
    composition
        .globals
        .azure_default_api_version
        .clone()
        .unwrap_or_else(|| AZURE_DEFAULT_API_VERSION.to_owned())
}

// the call's own key, litellm.api_key and litellm.azure_key, the three ranks both providers share
fn ranked_globals(composition: &Composition, credentials: &Credentials) -> Option<SecretString> {
    truthy_secret(credentials.api_key.as_ref())
        .or_else(|| truthy_secret(composition.globals.api_key.as_ref()))
        .or_else(|| truthy_secret(composition.globals.azure_key.as_ref()))
}

// api_key: the shared ranks, then AZURE_OPENAI_API_KEY and AZURE_API_KEY as they stand
fn ranked_key(composition: &Composition, credentials: &Credentials) -> Option<SecretString> {
    ranked_globals(composition, credentials)
        .or_else(|| {
            composition
                .lookup("AZURE_OPENAI_API_KEY")
                .filter(|key| !key.is_empty())
                .map(SecretString::from)
        })
        .or_else(|| composition.lookup("AZURE_API_KEY").map(SecretString::from))
}

// embedding reads only AZURE_API_KEY from the environment, unlike chat and text
fn ranked_embedding_key(
    composition: &Composition,
    credentials: &Credentials,
) -> Option<SecretString> {
    ranked_globals(composition, credentials)
        .or_else(|| composition.lookup("AZURE_API_KEY").map(SecretString::from))
}

fn api_type(composition: &Composition) -> String {
    composition
        .lookup("AZURE_API_TYPE")
        .filter(|kind| !kind.is_empty())
        .map_or_else(|| DEFAULT_API_TYPE.to_owned(), str::to_owned)
}

// optional_params.get("extra_body", {}).pop("azure_ad_token", None): the removal mutates the body's object
fn take_extra_body(
    credentials: &mut Credentials,
) -> Result<(Object, Option<String>), AzureProviderError> {
    let Some(value) = credentials.extra_body.take() else {
        return Ok((Object::new(), None));
    };
    let Json::Object(mut fields) = value else {
        return Err(AzureProviderError::ExtraBodyNotAnObject);
    };
    let token = match fields.shift_remove("azure_ad_token") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text).filter(|text| !text.is_empty()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(AzureProviderError::ExtraBodyToken);
        }
    };
    Ok((fields, token))
}

fn take_token(
    composition: &Composition,
    explicit: Option<&SecretString>,
    from_body: Option<String>,
) -> Option<SecretString> {
    from_body
        .map(SecretString::from)
        .filter(|token| !token.expose_secret().is_empty())
        .or_else(|| truthy_secret(explicit))
        .or_else(|| composition.lookup("AZURE_AD_TOKEN").map(SecretString::from))
}

// main.py merges kwargs.headers with kwargs.extra_headers, then falls back to litellm.headers
fn take_headers(
    composition: &Composition,
    credentials: &Credentials,
) -> (Option<Headers>, Option<Headers>, Option<Headers>) {
    let globals = composition
        .globals
        .headers
        .clone()
        .filter(|map| !map.is_empty());
    (
        credentials.headers.clone(),
        credentials.extra_headers.clone(),
        globals,
    )
}

pub fn resolve_chat(
    composition: &Composition,
    mut credentials: Credentials,
    detection_model: &str,
) -> Result<Resolved, AzureProviderError> {
    let api_base = ranked_base(composition, &credentials);
    let api_version = Some(
        ranked_version(composition, &credentials).unwrap_or_else(|| default_version(composition)),
    );
    let api_key = ranked_key(composition, &credentials);
    let auth_token = truthy_secret(credentials.azure_ad_token.as_ref());
    let (extra_body, from_body) = take_extra_body(&mut credentials)?;
    let azure_ad_token = take_token(composition, None, from_body);
    let (headers, extra_headers, globals_headers) = take_headers(composition, &credentials);
    Ok(Resolved {
        api_base,
        api_version,
        api_key,
        api_type: api_type(composition),
        auth_token,
        azure_ad_token,
        azure_ad_token_provider: credentials.azure_ad_token_provider.take(),
        headers,
        extra_headers,
        globals_headers,
        max_retries: credentials.max_retries.clone(),
        extra_body,
        default_api_version: default_version(composition),
        o_series: is_o_series_model(detection_model),
    })
}

pub fn resolve_text(
    composition: &Composition,
    mut credentials: Credentials,
) -> Result<Resolved, AzureProviderError> {
    let Some(api_base) = ranked_base(composition, &credentials) else {
        return Err(AzureProviderError::MissingTextApiBase);
    };
    let api_key = ranked_key(composition, &credentials);
    let auth_token = truthy_secret(credentials.azure_ad_token.as_ref());
    let (extra_body, from_body) = take_extra_body(&mut credentials)?;
    let azure_ad_token = take_token(composition, None, from_body);
    let (headers, extra_headers, globals_headers) = take_headers(composition, &credentials);
    Ok(Resolved {
        api_base: Some(api_base),
        api_version: ranked_version(composition, &credentials),
        api_key,
        api_type: api_type(composition),
        auth_token,
        azure_ad_token,
        azure_ad_token_provider: credentials.azure_ad_token_provider.take(),
        headers,
        extra_headers,
        globals_headers,
        max_retries: None,
        extra_body,
        default_api_version: default_version(composition),
        o_series: false,
    })
}

pub fn resolve_embedding(
    composition: &Composition,
    mut credentials: Credentials,
) -> Result<Resolved, AzureProviderError> {
    let Some(api_base) = ranked_base(composition, &credentials) else {
        return Err(AzureProviderError::MissingEmbeddingApiBase);
    };
    let api_version = Some(
        ranked_version(composition, &credentials).unwrap_or_else(|| default_version(composition)),
    );
    let auth_token = truthy_secret(credentials.azure_ad_token.as_ref());
    let azure_ad_token = take_token(composition, credentials.azure_ad_token.as_ref(), None);
    let (headers, extra_headers, globals_headers) = take_headers(composition, &credentials);
    // embeddings hand kwargs.extra_body straight to the SDK, which merges it into the request body
    let extra_body = match credentials.extra_body.take() {
        None | Some(Json::Null) => Object::new(),
        Some(Json::Object(fields)) => fields,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(AzureProviderError::ExtraBodyNotAnObject);
        }
    };
    Ok(Resolved {
        api_base: Some(api_base),
        api_version,
        api_key: ranked_embedding_key(composition, &credentials),
        api_type: api_type(composition),
        auth_token,
        azure_ad_token,
        azure_ad_token_provider: credentials.azure_ad_token_provider.take(),
        headers,
        extra_headers,
        globals_headers,
        max_retries: None,
        extra_body,
        default_api_version: default_version(composition),
        o_series: false,
    })
}

// Merge selected config defaults only for absent keys, so an explicit None stays None.
pub fn merge_defaults<T>(optional: &mut Object, defaults: &T) -> Result<(), AzureProviderError>
where
    T: Serialize + ?Sized,
{
    let values: Object = convert(defaults)?;
    for (name, value) in values {
        if value != Json::Null && !optional.contains_key(&name) {
            optional.insert(name, value);
        }
    }
    Ok(())
}
