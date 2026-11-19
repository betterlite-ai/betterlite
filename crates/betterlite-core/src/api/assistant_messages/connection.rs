use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_azure::azure_urls::AuthParams;
use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_openai::threads::records::ThreadMessageData;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;
use serde::Deserialize;
use serde_with::rust::double_option;

use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, request_timeout,
    resolve_fine_tuning_timeout,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::error::SetupError;
use super::options::{MessageData, MessageOptions, select_message_params};

// the kwargs the message wrappers read through GenericLiteLLMParams and get_litellm_params
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MessageParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub timeout: Option<MessageTimeout>,
    pub request_timeout: Option<f64>,
    pub max_retries: Option<i64>,
    pub azure_ad_token: Option<SecretString>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
    pub azure_scope: Option<String>,
    #[serde(default, with = "double_option")]
    pub extra_body: Option<Option<BTreeMap<String, Json>>>,
}

// GenericLiteLLMParams.timeout declares float | str | httpx.Timeout
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MessageTimeout {
    Plain(TimeoutValue),
    Structured(StructuredTimeout),
}

#[derive(Debug, Clone)]
pub struct Prepared {
    pub route: Route,
    pub data: Option<ThreadMessageData>,
    pub deadline: Duration,
    pub asynchronous: bool,
    pub max_retries: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum Route {
    OpenAi(OpenAiCredentials),
    Azure(AzureCredentials),
}

#[derive(Debug, Clone)]
pub struct OpenAiCredentials {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AzureCredentials {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    // the facade's own extra_body/general-token choice, which its provider client never reads
    pub facade_ad_token: Option<SecretString>,
    pub params: LiteLlmParams,
}

pub struct PrepareInput<'request> {
    pub provider: &'request str,
    pub operation: &'static str,
    // get_messages runs no parameter selection at all (main.py#L1063-L1065)
    pub data: Option<&'request MessageData>,
    pub params: &'request MessageParams,
    pub settings: &'request ResourceSdkSettings,
    pub drop_params: bool,
    pub asynchronous: bool,
}

pub fn prepare(input: &PrepareInput<'_>) -> Result<Prepared, SetupError> {
    // add_message selects its options before any provider client exists; get_messages selects none (main.py#L1063-L1065)
    let selected = match input.data {
        Some(message) => Some(select_message_params(&MessageOptions {
            role: Some(Json::Text(message.role.clone())),
            content: Some(message.content.clone()),
            attachments: message.attachments.clone(),
            metadata: message.metadata.clone(),
            provider: input.provider.to_owned(),
            drop_params: input.drop_params,
            kwargs: Object::new(),
        })?),
        None => None,
    };
    let timeout = resolve_timeout(input.params, input.provider)?;
    let deadline = deadline_of(&timeout)?;
    let route = route_of(input)?;
    Ok(Prepared {
        route,
        data: selected.as_ref().map(selected_data).transpose()?,
        deadline,
        asynchronous: input.asynchronous,
        max_retries: input
            .params
            .max_retries
            .and_then(|value| u32::try_from(value).ok()),
    })
}

fn route_of(input: &PrepareInput<'_>) -> Result<Route, SetupError> {
    let settings = input.settings;
    let options = &ResourceSdkRequestOptions::from(input.params);
    match input.provider {
        "openai" => Ok(Route::OpenAi(OpenAiCredentials {
            api_base: settings.openai_base(options),
            api_key: settings.openai_api_key(options).cloned(),
            organization: settings.openai_organization(options).map(str::to_owned),
        })),
        "azure" => Ok(Route::Azure(AzureCredentials {
            api_base: settings.azure_base(options).map(str::to_owned),
            api_version: settings.azure_version(options).map(str::to_owned),
            api_key: settings.azure_api_key(options).cloned(),
            facade_ad_token: facade_ad_token(input.params, settings),
            params: azure_params(input.params),
        })),
        other => Err(SetupError::UnsupportedProvider {
            provider: other.to_owned(),
            operation: input.operation,
        }),
    }
}

pub(crate) fn resolve_timeout(
    params: &MessageParams,
    provider: &str,
) -> Result<FineTuningTimeout, TimeoutError> {
    match params.timeout.as_ref() {
        Some(MessageTimeout::Structured(timeout)) => {
            resolve_fine_tuning_timeout(None, provider, Some(*timeout))
        }
        Some(MessageTimeout::Plain(value)) if !falls_through(value) => {
            resolve_fine_tuning_timeout(Some(value), provider, None)
        }
        // optional_params.timeout or request_timeout or 600, where a zero or an empty value falls through
        None | Some(MessageTimeout::Plain(_)) => Ok(FineTuningTimeout::Seconds(request_timeout(
            None,
            params.request_timeout,
        )?)),
    }
}

fn falls_through(timeout: &TimeoutValue) -> bool {
    match timeout {
        TimeoutValue::Number(seconds) => *seconds == 0.0_f64,
        TimeoutValue::String(text) => text.is_empty(),
    }
}

// httpx.Timeout has no single betterlite counterpart, so its read phase bounds the whole call
pub(crate) fn deadline_of(timeout: &FineTuningTimeout) -> Result<Duration, SetupError> {
    let seconds = match timeout {
        FineTuningTimeout::Seconds(seconds) => *seconds,
        FineTuningTimeout::Structured(structured) => structured
            .read
            .filter(|read| *read != 0.0_f64)
            .unwrap_or(600.0_f64),
    };
    Duration::try_from_secs_f64(seconds).map_err(|source| SetupError::Deadline { seconds, source })
}

impl From<&MessageParams> for ResourceSdkRequestOptions {
    fn from(params: &MessageParams) -> Self {
        Self {
            api_base: params.api_base.clone(),
            api_version: params.api_version.clone(),
            api_key: params.api_key.clone(),
            organization: params.organization.clone(),
            vertex_project: None,
            vertex_location: None,
            vertex_credentials: None,
        }
    }
}

fn selected_data(selected: &Object) -> Result<ThreadMessageData, SetupError> {
    let value = serde_json::to_value(Json::Object(selected.clone()))
        .map_err(|source| SetupError::Data { source })?;
    serde_json::from_value(value).map_err(|source| SetupError::Data { source })
}

// extra_body.pop("azure_ad_token") only when the mapping is not null; a null one reads AZURE_AD_TOKEN instead (main.py#L985-L991).
fn facade_ad_token(params: &MessageParams, settings: &ResourceSdkSettings) -> Option<SecretString> {
    match &params.extra_body {
        Some(Some(fields)) => match fields.get("azure_ad_token") {
            Some(Json::Text(text)) => Some(SecretString::from(text.clone())),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        },
        Some(None) => settings.azure_ad_token.clone(),
        None => None,
    }
}

fn azure_params(params: &MessageParams) -> LiteLlmParams {
    let auth = AuthParams {
        tenant_id: params.tenant_id.clone(),
        client_id: params.client_id.clone(),
        client_secret: params.client_secret.clone(),
        azure_username: params.azure_username.clone(),
        azure_password: params.azure_password.clone(),
        azure_scope: params.azure_scope.clone(),
    };
    LiteLlmParams {
        azure_ad_token: params.azure_ad_token.clone(),
        auth,
        max_retries: params.max_retries,
        timeout: raw_timeout(params.timeout.as_ref()),
    }
}

// get_litellm_params keeps the raw timeout keyword, which the Azure client reads instead of the resolved one.
fn raw_timeout(timeout: Option<&MessageTimeout>) -> Option<f64> {
    match timeout {
        Some(MessageTimeout::Plain(TimeoutValue::Number(seconds))) => Some(*seconds),
        Some(MessageTimeout::Plain(TimeoutValue::String(text))) => text.trim().parse::<f64>().ok(),
        Some(MessageTimeout::Structured(structured)) => Some(
            structured
                .read
                .filter(|read| *read != 0.0_f64)
                .unwrap_or(600.0_f64),
        ),
        None => None,
    }
}
