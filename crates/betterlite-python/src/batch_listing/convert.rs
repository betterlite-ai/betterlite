use std::collections::BTreeMap;

use betterlite_config::loader::ConfigError;
use betterlite_config::sections::Section;
use betterlite_core::api::batch_provider_cancel::connection::CancelTimeout;
use betterlite_core::api::batch_provider_cancel::{
    BatchProviderCancelInput, CancelBatchCall, CancelCall,
};
use betterlite_core::api::batch_provider_list::{BatchListTimeoutValue, BatchProviderListInput};
use betterlite_core::api::resource_sdk_settings::timeout::{StructuredTimeout, TimeoutValue};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::batches::records::BatchObject;
use betterlite_openai::batches::sdk::TimeoutPhases;
use pyo3::PyErr;
use pyo3::exceptions::PyValueError;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_with::{As, FromIntoRef};

mod tests;

const OPENAI_DEFAULT_BASE: &str = "https://api.openai.com";
const XAI_DEFAULT_BASE: &str = "https://api.x.ai/v1";
const LIST_SHAPES: &str = "seconds, a number written as text, or the phases of an httpx.Timeout";
const CANCEL_SHAPES: &str = "seconds, a number written as text, or the phases of an httpx.Timeout";

// Everything one Python session owns besides its HTTP client: the environment snapshot and the module globals.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SessionWire {
    pub(crate) environment: Option<BTreeMap<String, String>>,
    pub(crate) settings: ModuleSettings,
}

// the module globals a list or cancel call reads, under the names betterlite exposes them
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

// list_batches pops alist_batches and hands `_is_async` to the provider call
pub(crate) const fn list_input(
    mut input: BatchProviderListInput,
    mode: CallMode,
) -> BatchProviderListInput {
    input.alist_batches = matches!(mode, CallMode::Async);
    input
}

// acancel_batch hands cancel_batch the same call, marked async, with the wrapper's model kwarg empty
pub(crate) fn cancel_input(mut call: CancelBatchCall, mode: CallMode) -> CancelBatchCall {
    call.call = match mode {
        CallMode::Sync => CancelCall::Sync,
        CallMode::Async => CancelCall::Async {
            model_in_kwargs: None,
        },
    };
    call
}

// what a cancelled batch hands LiteLLMBatch._hidden_params: the provider, the model and the base the caller named
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

// the same policy create and retrieve use: the caller's base, else the provider's default when the source names one
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

// The batch the provider cancelled, plus the metadata betterlite hangs on the result it hands back.
#[derive(Debug, Serialize)]
pub(crate) struct CancelAnswer<'batch> {
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
    #[error("the batch payload's timeout is not usable: {0}")]
    Timeout(String),
    #[error("the batch payload's timeout is not usable: {0}")]
    Phases(#[source] serde_json::Error),
    #[error("the batch payload's timeout is not usable: expected {0}")]
    Shapes(&'static str),
    #[error(
        "this session lists and cancels batches; the async invoke status is B1038's call, not this one"
    )]
    Operation,
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
pub(crate) fn list_payload(text: &str) -> Result<BatchProviderListInput, ConvertError> {
    let (rewritten, given_timeout) = without_field(text, &["timeout"])?;
    let mut input: BatchProviderListInput = parse(&rewritten)?;
    input.timeout = given_timeout.as_ref().map(list_timeout).transpose()?;
    Ok(input)
}

pub(crate) fn cancel_payload(text: &str) -> Result<CancelBatchCall, ConvertError> {
    let (rewritten, given_timeout) = without_field(text, &["cancel", "params", "timeout"])?;
    let input: BatchProviderCancelInput = parse(&rewritten)?;
    let BatchProviderCancelInput::Cancel(mut call) = input else {
        return Err(ConvertError::Operation);
    };
    call.params.timeout = given_timeout.as_ref().map(cancel_timeout).transpose()?;
    Ok(*call)
}

fn without_field(text: &str, path: &[&str]) -> Result<(String, Option<JsonValue>), ConvertError> {
    let mut payload: JsonValue = parse(text)?;
    let taken = take_field(&mut payload, path);
    let rewritten =
        serde_json::to_string(&payload).map_err(|source| ConvertError::Write { source })?;
    Ok((rewritten, taken))
}

// the timeout a payload names, removed from where it sat so the typed parse below never sees it
fn take_field(payload: &mut JsonValue, path: &[&str]) -> Option<JsonValue> {
    let (first, rest) = path.split_first()?;
    let JsonValue::Object(fields) = payload else {
        return None;
    };
    if rest.is_empty() {
        return fields
            .shift_remove(*first)
            .filter(|value| !matches!(value, JsonValue::Null));
    }
    take_field(fields.get_mut(*first)?, rest)
}

fn seconds(number: &serde_json::Number) -> Result<f64, ConvertError> {
    number
        .as_f64()
        .ok_or_else(|| ConvertError::Timeout(format!("{number} is not a number of seconds")))
}

fn list_timeout(value: &JsonValue) -> Result<BatchListTimeoutValue, ConvertError> {
    match value {
        JsonValue::Number(number) => Ok(BatchListTimeoutValue::Seconds(seconds(number)?)),
        JsonValue::Text(text) => Ok(BatchListTimeoutValue::Text(text.clone())),
        JsonValue::Object(_) => Ok(BatchListTimeoutValue::Phases(phases_value(value)?)),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) => {
            Err(ConvertError::Shapes(LIST_SHAPES))
        }
    }
}

fn cancel_timeout(value: &JsonValue) -> Result<CancelTimeout, ConvertError> {
    if let JsonValue::Object(_) = value {
        let phases = phases_value(value)?;
        return Ok(CancelTimeout::Structured(StructuredTimeout {
            connect: phases.connect,
            read: phases.read,
            write: phases.write,
            pool: phases.pool,
        }));
    }
    if let JsonValue::Number(number) = value {
        return Ok(CancelTimeout::Plain(TimeoutValue::Number(seconds(number)?)));
    }
    if let JsonValue::Text(text) = value {
        return Ok(CancelTimeout::Plain(TimeoutValue::String(text.clone())));
    }
    Err(ConvertError::Shapes(CANCEL_SHAPES))
}

fn phases_value(value: &JsonValue) -> Result<TimeoutPhases, ConvertError> {
    serde_json::from_str(
        &serde_json::to_string(value).map_err(|source| ConvertError::Write { source })?,
    )
    .map_err(ConvertError::Phases)
}
