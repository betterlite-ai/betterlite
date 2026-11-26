use std::collections::BTreeMap;

use betterlite_core::api::file_provider_read::{Kwarg, ReadKwargs, ReadProvider};
use betterlite_core::api::file_provider_write::{WriteParams, WriteTimeout};
use betterlite_core::api::fine_tuning_read::{
    ReadStructuredTimeout, ReadTimeout, ReadTimeoutValue,
};
use betterlite_core::api::resource_sdk_settings::timeout::{StructuredTimeout, TimeoutValue};
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_models::controls::json_value::JsonValue;
use secrecy::SecretString;
use serde::Deserialize;

use super::ConvertError;

const TIMEOUT_SHAPES: &str = "seconds, a number written as text, or the phases of an httpx.Timeout";
const REQUEST_TIMEOUT_SHAPES: &str = "seconds or a number written as text";

// GenericLiteLLMParams plus the call's own kwargs, under the names the Rust inputs take
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ParamsWire {
    api_key: Option<SecretString>,
    azure_ad_token: Option<SecretString>,
    tenant_id: Option<SecretString>,
    client_id: Option<SecretString>,
    client_secret: Option<SecretString>,
    azure_username: Option<SecretString>,
    azure_password: Option<SecretString>,
    azure_scope: Option<SecretString>,
    api_base: Option<String>,
    api_version: Option<String>,
    organization: Option<String>,
    timeout: Option<JsonValue>,
    request_timeout: Option<JsonValue>,
    max_retries: Option<i64>,
    litellm_call_id: Option<String>,
    id: Option<String>,
    passthrough: Option<bool>,
}

pub(super) fn write_params(params: &ParamsWire) -> Result<WriteParams<(), ()>, ConvertError> {
    Ok(WriteParams {
        api_base: params.api_base.clone(),
        api_key: params.api_key.clone(),
        api_version: params.api_version.clone(),
        organization: params.organization.clone(),
        timeout: params.timeout.as_ref().map(write_timeout).transpose()?,
        request_timeout: params
            .request_timeout
            .as_ref()
            .map(plain_write_timeout)
            .transpose()?,
        max_retries: params.max_retries,
        passthrough: params.passthrough,
        litellm_call_id: params.litellm_call_id.clone().map(Some),
        id: params.id.clone(),
        logger: Some(()),
        client: None,
        credentials: Some(trusted_credentials(params)),
    })
}

pub(super) fn read_kwargs(
    params: ParamsWire,
    provider: Option<String>,
    extra_headers: Option<BTreeMap<String, String>>,
    extra_body: Option<BTreeMap<String, String>>,
) -> Result<(ReadKwargs, TrustedCredentials), ConvertError> {
    let trusted = trusted_credentials(&params);
    let timeout = params.timeout.as_ref().map(read_timeout).transpose()?;
    let request_timeout = params
        .request_timeout
        .as_ref()
        .map(plain_read_timeout)
        .transpose()?;
    let kwargs = ReadKwargs {
        custom_llm_provider: provider.map_or_else(ReadProvider::default, ReadProvider),
        extra_headers,
        extra_body,
        api_base: params.api_base,
        api_key: params.api_key,
        api_version: params.api_version,
        organization: params.organization,
        timeout,
        request_timeout,
        max_retries: params.max_retries,
        is_async: false,
        litellm_call_id: params.litellm_call_id.map_or(Kwarg::Absent, Kwarg::Text),
        id: params.id.map_or(Kwarg::Absent, Kwarg::Text),
        after: None,
        limit: None,
        order: None,
    };
    Ok((kwargs, trusted))
}

// GenericLiteLLMParams.timeout declares float | str | httpx.Timeout
fn read_timeout(value: &JsonValue) -> Result<ReadTimeout, ConvertError> {
    if let JsonValue::Object(_) = value {
        let text = serde_json::to_string(value).map_err(ConvertError::Phases)?;
        let phases: ReadStructuredTimeout =
            serde_json::from_str(&text).map_err(ConvertError::Phases)?;
        return Ok(ReadTimeout::Structured(phases));
    }
    plain_read_timeout(value)
        .map(ReadTimeout::Plain)
        .map_err(|_error| ConvertError::TimeoutShapes(TIMEOUT_SHAPES))
}

// the write inputs take the same timeout the read inputs declare, converted by the core's own TryFrom impls
fn write_timeout(value: &JsonValue) -> Result<WriteTimeout, ConvertError> {
    match read_timeout(value)? {
        ReadTimeout::Structured(phases) => StructuredTimeout::try_from(&phases)
            .map(WriteTimeout::Structured)
            .map_err(|_error| ConvertError::TimeoutShapes(TIMEOUT_SHAPES)),
        ReadTimeout::Plain(plain) => TimeoutValue::try_from(&plain)
            .map(WriteTimeout::Value)
            .map_err(|_error| ConvertError::TimeoutShapes(TIMEOUT_SHAPES)),
    }
}

fn plain_read_timeout(value: &JsonValue) -> Result<ReadTimeoutValue, ConvertError> {
    if let JsonValue::Number(number) = value {
        return Ok(ReadTimeoutValue::Number(number.clone()));
    }
    if let JsonValue::Text(text) = value {
        return Ok(ReadTimeoutValue::Text(text.clone()));
    }
    Err(ConvertError::TimeoutShapes(REQUEST_TIMEOUT_SHAPES))
}

fn plain_write_timeout(value: &JsonValue) -> Result<TimeoutValue, ConvertError> {
    let plain = plain_read_timeout(value)?;
    TimeoutValue::try_from(&plain)
        .map_err(|_error| ConvertError::TimeoutShapes(REQUEST_TIMEOUT_SHAPES))
}

// Python SDK keyword credentials are trusted call settings, not gateway request credentials.
fn trusted_credentials(params: &ParamsWire) -> TrustedCredentials {
    let fields = [
        ("azure_ad_token", &params.azure_ad_token),
        ("tenant_id", &params.tenant_id),
        ("client_id", &params.client_id),
        ("client_secret", &params.client_secret),
        ("azure_username", &params.azure_username),
        ("azure_password", &params.azure_password),
        ("azure_scope", &params.azure_scope),
    ];
    TrustedCredentials {
        values: fields
            .into_iter()
            .filter_map(|(key, value)| value.clone().map(|secret| (key.to_owned(), secret)))
            .collect(),
    }
}
