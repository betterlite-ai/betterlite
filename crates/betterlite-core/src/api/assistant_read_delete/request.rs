use betterlite_http::client_kind::ClientKind;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_with::rust::double_option;

use crate::api::resource_sdk_settings::ResourceSdkRequestOptions;
use crate::api::resource_sdk_settings::timeout::TimeoutValue;

// serde's adapter builds this private, non-Debug value only while a model is serialized
#[derive(Serialize)]
#[serde(transparent)]
struct ExposedSecret(String);

impl From<&SecretString> for ExposedSecret {
    fn from(secret: &SecretString) -> Self {
        Self(secret.expose_secret().to_owned())
    }
}

type SecretWire = Option<serde_with::FromIntoRef<ExposedSecret>>;

// get/aget_assistants and delete/adelete_assistant
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantOperation {
    List,
    Delete,
}

impl AssistantOperation {
    #[must_use]
    pub const fn function(self) -> &'static str {
        match self {
            Self::List => "get_assistants",
            Self::Delete => "delete_assistant",
        }
    }
}

// the async wrappers set their own flag to True before the sync function reads it
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantWrapper {
    #[default]
    Sync,
    Async,
}

// GenericLiteLLMParams.timeout declares float | str | httpx.Timeout
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AssistantTimeout {
    Number(f64),
    Text(String),
    Structured(TimeoutPhases),
}

impl AssistantTimeout {
    #[must_use]
    pub fn value(&self) -> Option<TimeoutValue> {
        match self {
            Self::Number(value) if *value != 0.0 => Some(TimeoutValue::Number(*value)),
            Self::Text(text) if !text.is_empty() => Some(TimeoutValue::String(text.clone())),
            Self::Number(_) | Self::Text(_) | Self::Structured(_) => None,
        }
    }
}

// the four httpx.Timeout phases, local because the shared StructuredTimeout doesn't serialize
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct TimeoutPhases {
    #[serde(default)]
    pub connect: Option<f64>,
    #[serde(default)]
    pub read: Option<f64>,
    #[serde(default)]
    pub write: Option<f64>,
    #[serde(default)]
    pub pool: Option<f64>,
}

// the class of a client the caller passed in: LiteLLM reads it before and after the call
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuppliedClientKind {
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "async_openai")]
    AsyncOpenAi,
    #[serde(rename = "azure_openai")]
    AzureOpenAi,
    #[serde(rename = "async_azure_openai")]
    AsyncAzureOpenAi,
}

impl SuppliedClientKind {
    #[must_use]
    pub const fn client_kind(self) -> ClientKind {
        match self {
            Self::OpenAi | Self::AzureOpenAi => ClientKind::Sync,
            Self::AsyncOpenAi | Self::AsyncAzureOpenAi => ClientKind::Asynchronous,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuppliedClient {
    pub kind: SuppliedClientKind,
    // the SDK the caller built: its own base, key and api_version, never the facade's options
    pub api_base: Option<String>,
    #[serde(serialize_with = "serde_with::As::<SecretWire>::serialize")]
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
}

// the kwargs GenericLiteLLMParams and get_litellm_params read
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AssistantParams {
    #[serde(serialize_with = "serde_with::As::<SecretWire>::serialize")]
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub max_retries: Option<i64>,
    pub timeout: Option<AssistantTimeout>,
    pub request_timeout: Option<f64>,
    #[serde(serialize_with = "serde_with::As::<SecretWire>::serialize")]
    pub azure_ad_token: Option<SecretString>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    #[serde(serialize_with = "serde_with::As::<SecretWire>::serialize")]
    pub client_secret: Option<SecretString>,
    pub azure_username: Option<String>,
    #[serde(serialize_with = "serde_with::As::<SecretWire>::serialize")]
    pub azure_password: Option<SecretString>,
    pub azure_scope: Option<String>,
}

impl From<&AssistantParams> for ResourceSdkRequestOptions {
    fn from(params: &AssistantParams) -> Self {
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

// one call of one of the four functions, as its arguments arrive
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantReadDeleteInput {
    pub operation: AssistantOperation,
    // LiteLLM declares no default: the provider argument is required and any other value answers 400
    pub provider: String,
    pub assistant_id: Option<String>,
    #[serde(default)]
    pub entry: AssistantWrapper,
    // aget_assistants and async_delete_assistants declare bool | None, and LiteLLM refuses any other JSON
    pub aget_assistants: Option<Json>,
    pub async_delete_assistants: Option<Json>,
    // optional_params.get("extra_body", {}): omitted reads as {}, while an explicit null stays None
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub extra_body: Option<Option<IndexMap<String, Json>>>,
    pub client: Option<SuppliedClient>,
    // get_assistants accepts the pagination kwargs and never forwards them to the provider method
    pub after: Option<String>,
    pub before: Option<String>,
    pub limit: Option<i64>,
    pub order: Option<String>,
    #[serde(default)]
    pub params: AssistantParams,
}
