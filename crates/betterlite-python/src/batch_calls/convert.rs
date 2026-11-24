use std::collections::BTreeMap;

use betterlite_config::loader::ConfigError;
use betterlite_config::sections::Section;
use betterlite_core::api::batch_provider_create::{
    BatchProviderCreateCall, BatchProviderCreateInput,
};
use betterlite_core::api::batch_provider_read::BatchProviderReadInput;
use betterlite_core::api::batch_provider_read::call::BatchProviderReadCall;
use betterlite_openai::batches::records::BatchObject;
use pyo3::PyErr;
use pyo3::exceptions::PyValueError;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_with::{As, FromIntoRef};

pub(crate) mod request;
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
    #[serde(serialize_with = "As::<Option<FromIntoRef<ExposedSecret>>>::serialize")]
    pub(crate) api_key: Option<SecretString>,
    #[serde(serialize_with = "As::<Option<FromIntoRef<ExposedSecret>>>::serialize")]
    pub(crate) openai_key: Option<SecretString>,
    #[serde(serialize_with = "As::<Option<FromIntoRef<ExposedSecret>>>::serialize")]
    pub(crate) azure_key: Option<SecretString>,
    pub(crate) api_version: Option<String>,
    pub(crate) organization: Option<String>,
    pub(crate) vertex_project: Option<String>,
    pub(crate) vertex_location: Option<String>,
}

// serde's adapter builds this private, non-Debug value only while the module settings are serialized
#[derive(Serialize)]
#[serde(transparent)]
struct ExposedSecret(String);

impl From<&SecretString> for ExposedSecret {
    fn from(secret: &SecretString) -> Self {
        Self(secret.expose_secret().to_owned())
    }
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

// one section's settings: the module globals become its Raw shape, then its own fallback chain runs
pub(crate) fn section_settings<Raw>(
    module: &ModuleSettings,
    environment: &BTreeMap<String, String>,
) -> Result<Raw::Settings, SettingsError>
where
    Raw: Section,
{
    let text = serde_json::to_string(&SectionModule {
        litellm_settings: module,
    })
    .map_err(SettingsError::Encode)?;
    let raw: Raw = serde_json::from_str(&text).map_err(SettingsError::Decode)?;
    raw.convert(environment).map_err(SettingsError::Config)
}

// Whether the call is the synchronous function or its awaitable twin, which is LiteLLM's `_is_async`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallMode {
    Sync,
    Async,
}

pub(crate) const fn create_input(
    mut input: BatchProviderCreateInput,
    mode: CallMode,
) -> BatchProviderCreateInput {
    input.is_async = matches!(mode, CallMode::Async);
    input
}

pub(crate) const fn read_input(
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

impl HiddenParams {
    pub(crate) fn new(provider: &str, model: Option<&str>, call: FinishedCall<'_>) -> Self {
        Self {
            llm_provider: provider.to_owned(),
            // LiteLLM takes model_id from model_info.id, which the Python facade writes into this key after the call
            model_id: None,
            litellm_model_name: model.map(str::to_owned),
            api_base: call.api_base(),
        }
    }
}

// whichever call a finished service hands back: its base is the hidden api_base
pub(crate) enum FinishedCall<'call> {
    Create(&'call BatchProviderCreateCall),
    Read(&'call BatchProviderReadCall),
}

impl FinishedCall<'_> {
    pub(crate) fn api_base(self) -> Option<String> {
        match self {
            Self::Create(call) => match call {
                BatchProviderCreateCall::ProviderConfig(inner) => inner.api_base.clone(),
                BatchProviderCreateCall::Xai(inner)
                | BatchProviderCreateCall::OpenAiCompatible(inner)
                | BatchProviderCreateCall::Azure(inner)
                | BatchProviderCreateCall::VertexAi(inner) => inner.api_base.clone(),
            },
            Self::Read(call) => match call {
                BatchProviderReadCall::OpenAiCompatible(inner) => Some(inner.api_base.clone()),
                BatchProviderReadCall::VertexAi(inner) => Some(inner.api_base.clone()),
                BatchProviderReadCall::ProviderConfig(inner) => inner.api_base.clone(),
                BatchProviderReadCall::Xai(inner) => inner.api_base.clone(),
                BatchProviderReadCall::Azure(inner) => inner.api_base.clone(),
                BatchProviderReadCall::Anthropic(inner) => inner.api_base.clone(),
                BatchProviderReadCall::BedrockAsyncInvoke(_)
                | BatchProviderReadCall::BedrockModelInvocationJob(_) => None,
            },
        }
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
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(ConvertError::Json { source }),
    }
}
