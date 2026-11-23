use std::collections::BTreeMap;

use betterlite_config::loader::ConfigError;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::{batch_provider_create, batch_provider_read, xai_identity};
use betterlite_core::api::batch_provider_create::{
    BatchProviderCreateCall, BatchProviderCreateInput,
};
use betterlite_core::api::batch_provider_read::BatchProviderReadInput;
use betterlite_core::api::batch_provider_read::call::BatchProviderReadCall;
use betterlite_openai::batches::records::BatchObject;
use pyo3::PyErr;
use pyo3::exceptions::PyValueError;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub(crate) mod request;
#[cfg(test)]
mod tests;

// Everything one Python session owns besides its HTTP client: the environment snapshot and the module globals.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SessionWire {
    pub(crate) environment: Option<BTreeMap<String, String>>,
    pub(crate) settings: ModuleSettings,
}

// the module globals a batch call reads, under the names betterlite exposes them
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ModuleSettings {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
    pub(crate) azure_key: Option<SecretString>,
    pub(crate) api_version: Option<String>,
    pub(crate) organization: Option<String>,
    pub(crate) vertex_project: Option<String>,
    pub(crate) vertex_location: Option<String>,
}

// the section parsers own the fallback chains and the os.environ/ references, so the module values go back through them
#[derive(Serialize)]
struct SectionModule<'module> {
    litellm_settings: &'module ModuleSettings,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SettingsError {
    #[error("the batch settings could not be written: {0}")]
    Encode(serde_json::Error),
    #[error("the batch settings could not be read back: {0}")]
    Decode(serde_json::Error),
    #[error(transparent)]
    Config(#[from] ConfigError),
}

fn section_raw<Raw>(module: &ModuleSettings) -> Result<Raw, SettingsError>
where
    Raw: DeserializeOwned,
{
    let text = serde_json::to_string(&SectionModule {
        litellm_settings: module,
    })
    .map_err(SettingsError::Encode)?;
    serde_json::from_str(&text).map_err(SettingsError::Decode)
}

pub(crate) fn create_settings(
    module: &ModuleSettings,
    environment: &BTreeMap<String, String>,
) -> Result<batch_provider_create::Settings, SettingsError> {
    let raw: batch_provider_create::Raw = section_raw(module)?;
    raw.convert(environment).map_err(SettingsError::Config)
}

pub(crate) fn read_settings(
    module: &ModuleSettings,
    environment: &BTreeMap<String, String>,
) -> Result<batch_provider_read::Settings, SettingsError> {
    let raw: batch_provider_read::Raw = section_raw(module)?;
    raw.convert(environment).map_err(SettingsError::Config)
}

pub(crate) fn xai_settings(
    module: &ModuleSettings,
    environment: &BTreeMap<String, String>,
) -> Result<xai_identity::Settings, SettingsError> {
    let raw: xai_identity::Raw = section_raw(module)?;
    raw.convert(environment).map_err(SettingsError::Config)
}

// Whether the call is the synchronous function or its awaitable twin, which is LiteLLM's `_is_async`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallMode {
    Sync,
    Async,
}

pub(crate) fn create_input(
    mut input: BatchProviderCreateInput,
    mode: CallMode,
) -> BatchProviderCreateInput {
    input.is_async = matches!(mode, CallMode::Async);
    input
}

pub(crate) fn read_input(
    mut input: BatchProviderReadInput,
    mode: CallMode,
) -> BatchProviderReadInput {
    input.aretrieve_batch = matches!(mode, CallMode::Async);
    input
}

// what a finished call hands LiteLLMBatch._hidden_params: the provider, the model and the base it reached
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct HiddenParams {
    pub(crate) llm_provider: String,
    pub(crate) model_id: Option<String>,
    pub(crate) litellm_model_name: Option<String>,
    pub(crate) api_base: Option<String>,
}

fn hidden_params(
    provider: String,
    model: Option<String>,
    api_base: Option<String>,
) -> HiddenParams {
    HiddenParams {
        llm_provider: provider,
        // LiteLLM takes model_id from model_info.id, which the Python facade writes into this key after the call
        model_id: None,
        litellm_model_name: model,
        api_base,
    }
}

pub(crate) fn create_hidden(
    input: &BatchProviderCreateInput,
    call: &BatchProviderCreateCall,
) -> HiddenParams {
    hidden_params(
        input.custom_llm_provider.as_str().to_owned(),
        input.model.clone(),
        create_call_base(call),
    )
}

fn create_call_base(call: &BatchProviderCreateCall) -> Option<String> {
    match call {
        BatchProviderCreateCall::ProviderConfig(inner) => inner.api_base.clone(),
        BatchProviderCreateCall::Xai(inner)
        | BatchProviderCreateCall::OpenAiCompatible(inner)
        | BatchProviderCreateCall::Azure(inner)
        | BatchProviderCreateCall::VertexAi(inner) => inner.api_base.clone(),
    }
}

pub(crate) fn read_hidden(
    input: &BatchProviderReadInput,
    call: &BatchProviderReadCall,
) -> HiddenParams {
    hidden_params(
        input.provider.name().to_owned(),
        input.model.clone(),
        read_call_base(call),
    )
}

fn read_call_base(call: &BatchProviderReadCall) -> Option<String> {
    match call {
        BatchProviderReadCall::ProviderConfig(inner) => inner.api_base.clone(),
        BatchProviderReadCall::Xai(inner) => inner.api_base.clone(),
        BatchProviderReadCall::OpenAiCompatible(inner) => Some(inner.api_base.clone()),
        BatchProviderReadCall::Azure(inner) => inner.api_base.clone(),
        BatchProviderReadCall::VertexAi(inner) => Some(inner.api_base.clone()),
        BatchProviderReadCall::Anthropic(inner) => inner.api_base.clone(),
        BatchProviderReadCall::BedrockAsyncInvoke(_)
        | BatchProviderReadCall::BedrockModelInvocationJob(_) => None,
    }
}

// The batch the provider answered with, plus the metadata LiteLLM hangs on the result it hands back.
#[derive(Debug, Serialize)]
pub(crate) struct BatchAnswer<'batch> {
    #[serde(flatten)]
    pub(crate) batch: &'batch BatchObject,
    #[serde(rename = "_hidden_params")]
    pub(crate) hidden_params: HiddenParams,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the batch payload is not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        PyValueError::new_err(error.to_string())
    }
}

// the one boundary every native method crosses: the text into the shape that method takes
pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    serde_json::from_str(text).map_err(|source| ConvertError::Json { source })
}
