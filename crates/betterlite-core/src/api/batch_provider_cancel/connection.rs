use std::num::ParseFloatError;

use betterlite_config::sections::batch_provider_cancel::Settings;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{CancelBatchCall, CancelProvider};

pub const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;
pub const DEFAULT_OPENAI_BASE: &str = "https://api.openai.com/v1";

// the kwargs cancel_batch reads through GenericLiteLLMParams and get_litellm_params
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CancelParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub aws_region_name: Option<String>,
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
    pub timeout: Option<TimeoutValue>,
    pub request_timeout: Option<f64>,
    pub max_retries: Option<i64>,
}

// GenericLiteLLMParams.timeout declares float | str | httpx.Timeout
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TimeoutValue {
    Seconds(f64),
    Text(String),
    Structured(StructuredTimeout),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StructuredTimeout {
    pub connect: Option<f64>,
    pub read: Option<f64>,
    pub write: Option<f64>,
    pub pool: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CancelTimeout {
    Seconds(f64),
    Structured(StructuredTimeout),
}

#[derive(Debug, Error)]
pub enum TimeoutValueError {
    #[error("the timeout {value} is not a number of seconds")]
    Invalid {
        value: String,
        #[source]
        source: ParseFloatError,
    },
}

// supports_httpx_timeout names openai, azure and bedrock; every other provider reads the timeout's read value
pub const fn supports_httpx_timeout(provider: &CancelProvider) -> bool {
    matches!(
        provider,
        CancelProvider::Openai | CancelProvider::Azure | CancelProvider::Bedrock
    )
}

#[must_use]
pub fn resolve_timeout(call: &CancelBatchCall) -> Result<CancelTimeout, TimeoutValueError> {
    let configured = match &call.params.timeout {
        Some(value) => Some(timeout_from(value)?),
        None => call.params.request_timeout.map(CancelTimeout::Seconds),
    };
    let timeout = configured.unwrap_or(CancelTimeout::Seconds(DEFAULT_TIMEOUT_SECONDS));
    match timeout {
        // httpx.Timeout.read or 600: a structured timeout a provider cannot take collapses to its read value
        CancelTimeout::Structured(structured) if !supports_httpx_timeout(&call.provider) => Ok(
            CancelTimeout::Seconds(structured.read.unwrap_or(DEFAULT_TIMEOUT_SECONDS)),
        ),
        other => Ok(other),
    }
}

fn timeout_from(value: &TimeoutValue) -> Result<CancelTimeout, TimeoutValueError> {
    match value {
        TimeoutValue::Seconds(seconds) => Ok(CancelTimeout::Seconds(*seconds)),
        TimeoutValue::Structured(structured) => Ok(CancelTimeout::Structured(*structured)),
        TimeoutValue::Text(text) => text
            .trim()
            .parse::<f64>()
            .map(CancelTimeout::Seconds)
            .map_err(|source| TimeoutValueError::Invalid {
                value: text.clone(),
                source,
            }),
    }
}

#[must_use]
pub fn openai_base(params: &CancelParams, settings: &Settings) -> String {
    let candidates = [
        params.api_base.as_deref(),
        settings.api_base.as_deref(),
        settings.openai_base_url.as_deref(),
        settings.openai_api_base.as_deref(),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .unwrap_or(DEFAULT_OPENAI_BASE)
        .to_owned()
}

#[must_use]
// the source's `a or b or c` chains, where an empty value counts as unset
macro_rules! fallback_accessors {
    ($value:ty, $borrow:ident, $empty:expr; $(
        $method:ident: $request_field:ident => [$($setting:ident),*];
    )+) => {$(
        #[must_use]
        pub fn $method<'value>(
            params: &'value CancelParams,
            settings: &'value Settings,
        ) -> Option<&'value $value> {
            let candidates = [
                params.$request_field.$borrow(),
                $(settings.$setting.$borrow(),)*
            ];
            candidates
                .into_iter()
                .flatten()
                .find(|value| !($empty)(value))
        }
    )+};
}

fallback_accessors! {
    str, as_deref, str::is_empty;
    openai_organization: organization => [organization, openai_organization];
    azure_base: api_base => [api_base, azure_api_base];
    azure_version: api_version => [api_version, azure_api_version];
    vertex_project: vertex_project => [vertex_project, vertexai_project];
    vertex_location: vertex_location => [vertex_location, vertexai_location];
}

fallback_accessors! {
    SecretString, as_ref, |value: &SecretString| value.expose_secret().is_empty();
    openai_api_key: api_key => [api_key, openai_key, openai_api_key];
    azure_api_key: api_key => [api_key, azure_key, azure_openai_api_key, azure_api_key];
    vertex_credentials: vertex_credentials => [vertexai_credentials];
}
