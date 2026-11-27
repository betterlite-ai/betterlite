use std::collections::BTreeMap;

use betterlite_config::loader::ConfigError;
use betterlite_config::sections::Section;
use betterlite_core::api::batch_provider_create::BatchProviderCreateInput;
use betterlite_core::api::batch_provider_read::BatchProviderReadInput;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::batches::records::BatchObject;
use indexmap::IndexMap;
use pyo3::PyErr;
use pyo3::exceptions::PyValueError;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_with::{As, FromIntoRef};

mod tests;
mod timeout;

const OPENAI_DEFAULT_BASE: &str = "https://api.openai.com";
const XAI_DEFAULT_BASE: &str = "https://api.x.ai/v1";

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

// what a finished call hands LiteLLMBatch._hidden_params: the provider, the model and the base the caller's kwargs name
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct HiddenParams {
    pub(crate) llm_provider: String,
    pub(crate) model_id: Option<String>,
    pub(crate) litellm_model_name: Option<String>,
    pub(crate) api_base: Option<String>,
}

impl HiddenParams {
    pub(crate) fn new(provider: &str, model: Option<&str>, given_api_base: Option<&str>) -> Self {
        Self {
            llm_provider: provider.to_owned(),
            // LiteLLM takes model_id from model_info.id, which the Python facade writes into this key after the call
            model_id: None,
            litellm_model_name: model.map(str::to_owned),
            api_base: hidden_api_base(provider, model, given_api_base),
        }
    }
}

// set_hidden_params reports get_api_base over the call's own kwargs; xai is only ever passed explicitly, while openai is also the facade's default and resolves nothing without a model
fn hidden_api_base(provider: &str, model: Option<&str>, given: Option<&str>) -> Option<String> {
    if let Some(api_base) = given {
        return Some(api_base.to_owned());
    }
    match (provider, model) {
        ("openai", Some(_)) => Some(String::from(OPENAI_DEFAULT_BASE)),
        ("xai", _) => Some(String::from(XAI_DEFAULT_BASE)),
        _ => None,
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
    #[error("the batch payload could not be written back: {source}")]
    Write {
        #[source]
        source: serde_json::Error,
    },
    #[error("the batch payload's optional_params.timeout is not usable: {0}")]
    Timeout(String),
    #[error("the batch payload's optional_params.timeout is not usable: {0}")]
    Phases(#[source] serde_json::Error),
    #[error("the batch payload's optional_params.timeout is not usable: expected {0}")]
    Shapes(&'static str),
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

// arbitrary_precision hides numbers from untagged enums, so the timeout is lifted out and read on its own
pub(crate) fn create_payload(text: &str) -> Result<BatchProviderCreateInput, ConvertError> {
    let (rewritten, given_timeout) = without_timeout(text)?;
    let mut input: BatchProviderCreateInput = parse(&rewritten)?;
    input.optional_params.timeout = given_timeout.as_ref().map(timeout::create).transpose()?;
    Ok(input)
}

pub(crate) fn read_payload(text: &str) -> Result<BatchProviderReadInput, ConvertError> {
    let (rewritten, given_timeout) = without_timeout(text)?;
    let mut input: BatchProviderReadInput = parse(&rewritten)?;
    input.optional_params.timeout = given_timeout.as_ref().map(timeout::read).transpose()?;
    Ok(input)
}

fn without_timeout(text: &str) -> Result<(String, Option<JsonValue>), ConvertError> {
    let mut payload: JsonValue = parse(text)?;
    let timeout = if let JsonValue::Object(fields) = &mut payload {
        take_optional_timeout(fields)
    } else {
        None
    };
    let rewritten =
        serde_json::to_string(&payload).map_err(|source| ConvertError::Write { source })?;
    Ok((rewritten, timeout))
}

fn take_optional_timeout(fields: &mut IndexMap<String, JsonValue>) -> Option<JsonValue> {
    let JsonValue::Object(optional) = fields.get_mut("optional_params")? else {
        return None;
    };
    optional
        .shift_remove("timeout")
        .filter(|value| !matches!(value, JsonValue::Null))
}
