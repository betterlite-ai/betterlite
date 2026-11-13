use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::destination::{
    OtelHeaders, build_langfuse_otel_config, build_langfuse_otel_headers,
    langfuse_authorization_header, langfuse_otel_host,
};
use super::error::LangfuseOtelError;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;

const PUBLIC_KEY_FIELD: &str = "langfuse_public_key";
const SECRET_KEY_FIELD: &str = "langfuse_secret_key";
const HOST_FIELD: &str = "langfuse_host";

const fn params_fields(
    params: &StandardCallbackDynamicParams,
) -> Result<Option<&IndexMap<String, Json>>, LangfuseOtelError> {
    match params {
        Json::Null => Ok(None),
        Json::Object(fields) => Ok(Some(fields)),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(LangfuseOtelError::DynamicParamsType)
        }
    }
}

// An empty string counts as absent, like the truthiness check LiteLLM runs on the credentials.
fn text_field<'fields>(
    fields: &'fields IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<&'fields str>, LangfuseOtelError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.as_str()).filter(|value| !value.is_empty())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(LangfuseOtelError::DynamicFieldType { field })
        }
    }
}

fn dynamic_credentials(
    fields: &IndexMap<String, Json>,
) -> Result<Option<(&str, SecretString)>, LangfuseOtelError> {
    let public_key = text_field(fields, PUBLIC_KEY_FIELD)?;
    let secret_key = text_field(fields, SECRET_KEY_FIELD)?;
    Ok(public_key.zip(secret_key.map(SecretString::from)))
}

pub fn construct_dynamic_otel_headers(
    params: &StandardCallbackDynamicParams,
) -> Result<OtelHeaders, LangfuseOtelError> {
    let Some(fields) = params_fields(params)? else {
        return Ok(OtelHeaders::new());
    };
    let Some((public_key, secret_key)) = dynamic_credentials(fields)? else {
        return Ok(OtelHeaders::new());
    };
    let auth_header = langfuse_authorization_header(public_key, &secret_key);
    Ok(build_langfuse_otel_headers(auth_header))
}

// Dynamic credentials define the whole export target, so a proxy without LANGFUSE_* env vars still reaches Langfuse.
pub fn construct_dynamic_otel_config(
    params: &StandardCallbackDynamicParams,
    env: &BTreeMap<String, String>,
) -> Result<Option<OpenTelemetryConfig>, LangfuseOtelError> {
    let Some(fields) = params_fields(params)? else {
        return Ok(None);
    };
    let Some((public_key, secret_key)) = dynamic_credentials(fields)? else {
        return Ok(None);
    };
    let langfuse_host = text_field(fields, HOST_FIELD)?
        .map(str::to_owned)
        .or_else(|| langfuse_otel_host(env));
    Ok(Some(build_langfuse_otel_config(
        public_key,
        &secret_key,
        langfuse_host.as_deref(),
        env,
    )))
}
