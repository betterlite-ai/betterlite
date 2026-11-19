use betterlite_openai::assistants::client::ClientKind;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Serialize};

use crate::api::resource_sdk_settings::ResourceSdkRequestOptions;
use crate::api::resource_sdk_settings::timeout::{StructuredTimeout, TimeoutValue};

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

    #[must_use]
    pub const fn flag(self) -> &'static str {
        match self {
            Self::List => "aget_assistants",
            Self::Delete => "async_delete_assistants",
        }
    }

    // the sync get raises a plain Exception where delete raises ValueError
    #[must_use]
    pub const fn flag_class(self) -> &'static str {
        match self {
            Self::List => "Exception",
            Self::Delete => "ValueError",
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

// aget_assistants and async_delete_assistants declare bool | None, and LiteLLM refuses any other JSON
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AssistantFlag {
    Bool(bool),
    Other(Json),
}

// GenericLiteLLMParams.timeout declares float | str | httpx.Timeout
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum AssistantTimeout {
    Plain(TimeoutValue),
    Structured(StructuredTimeout),
}

impl Serialize for AssistantTimeout {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Plain(TimeoutValue::Number(value)) => serializer.serialize_f64(*value),
            Self::Plain(TimeoutValue::String(text)) => serializer.serialize_str(text),
            Self::Structured(timeout) => {
                let mut state = serializer.serialize_struct("AssistantTimeout", 4)?;
                state.serialize_field("connect", &timeout.connect)?;
                state.serialize_field("read", &timeout.read)?;
                state.serialize_field("write", &timeout.write)?;
                state.serialize_field("pool", &timeout.pool)?;
                state.end()
            }
        }
    }
}

// optional_params.get("extra_body", {}): an omitted kwarg reads as {}, while an explicit null stays None
#[derive(Debug, Clone, Default, PartialEq)]
pub enum ExtraBody {
    #[default]
    Missing,
    Null,
    Body(IndexMap<String, Json>),
}

impl ExtraBody {
    #[must_use]
    pub const fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}

impl Serialize for ExtraBody {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Missing | Self::Null => serializer.serialize_none(),
            Self::Body(body) => body.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ExtraBody {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ExtraBodyVisitor;

        impl<'de> serde::de::Visitor<'de> for ExtraBodyVisitor {
            type Value = ExtraBody;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("the extra_body kwarg, null, or nothing")
            }

            fn visit_none<E>(self) -> Result<ExtraBody, E> {
                Ok(ExtraBody::Null)
            }

            fn visit_unit<E>(self) -> Result<ExtraBody, E> {
                Ok(ExtraBody::Null)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<ExtraBody, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                IndexMap::<String, Json>::deserialize(deserializer).map(ExtraBody::Body)
            }
        }

        deserializer.deserialize_option(ExtraBodyVisitor)
    }
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
            Self::AsyncOpenAi | Self::AsyncAzureOpenAi => ClientKind::Async,
        }
    }

    // isinstance(client, OpenAI) is true for the AzureOpenAI subclass only
    #[must_use]
    pub const fn is_openai_instance(self) -> bool {
        matches!(self, Self::OpenAi | Self::AzureOpenAi)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuppliedClient {
    pub kind: SuppliedClientKind,
    // the SDK the caller built: its own base, key and api_version, never the facade's options
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub api_version: Option<String>,
}

// the kwargs GenericLiteLLMParams and get_litellm_params read
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AssistantParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub max_retries: Option<i64>,
    pub timeout: Option<AssistantTimeout>,
    pub request_timeout: Option<f64>,
    pub azure_ad_token: Option<SecretString>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_username: Option<String>,
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
    #[serde(default)]
    pub assistant_id: Option<String>,
    #[serde(default)]
    pub entry: AssistantWrapper,
    #[serde(default)]
    pub aget_assistants: Option<AssistantFlag>,
    #[serde(default)]
    pub async_delete_assistants: Option<AssistantFlag>,
    #[serde(default, skip_serializing_if = "ExtraBody::is_missing")]
    pub extra_body: ExtraBody,
    #[serde(default)]
    pub client: Option<SuppliedClient>,
    // get_assistants accepts the pagination kwargs and never forwards them to the provider method
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub order: Option<String>,
    #[serde(default)]
    pub params: AssistantParams,
}
