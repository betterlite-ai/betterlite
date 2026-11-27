use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use pyo3::PyErr;
use pyo3::exceptions::PyValueError;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Number;

mod tests;

// The process settings one session reads: no config file, so the environment is the whole input.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SessionWire {
    pub(crate) environment: Option<BTreeMap<String, String>>,
    pub(crate) settings: ModuleSettings,
}

// the betterlite module globals the OpenAI evals config falls back to, under the names it reads
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ModuleSettings {
    pub(crate) api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
}

// A request argument outside the type LiteLLM declares: docs/compatibility.md answers it with a 400 naming the field.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the eval call payload is not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the {name} field does not have the declared type")]
    FieldType {
        provider: String,
        name: &'static str,
    },
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        PyValueError::new_err(error.to_string())
    }
}

// One call's arguments before they are split into the parameters every operation shares.
pub(crate) type Fields = IndexMap<String, Json>;

pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    serde_json::from_str(text).map_err(|source| ConvertError::Json { source })
}

pub(crate) fn parse_fields(text: &str) -> Result<Fields, ConvertError> {
    parse(text)
}

// an absent or null optional field takes its default; any other value must read as the declared type
pub(crate) fn take<T>(
    fields: &mut Fields,
    name: &'static str,
    provider: &str,
) -> Result<Option<T>, ConvertError>
where
    T: DeserializeOwned,
{
    match fields.shift_remove(name) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => decode(name, provider, value).map(Some),
    }
}

// a required field is never absent, so a null in one is the same type error
pub(crate) fn required<T>(
    fields: &mut Fields,
    name: &'static str,
    provider: &str,
) -> Result<T, ConvertError>
where
    T: DeserializeOwned,
{
    take(fields, name, provider)?.ok_or_else(|| field(provider, name))
}

fn decode<T>(name: &'static str, provider: &str, value: Json) -> Result<T, ConvertError>
where
    T: DeserializeOwned,
{
    let raw = serde_json::to_value(value).map_err(|source| ConvertError::Json { source })?;
    serde_json::from_value(raw).map_err(|_source| field(provider, name))
}

fn field(provider: &str, name: &'static str) -> ConvertError {
    ConvertError::FieldType {
        provider: provider.to_owned(),
        name,
    }
}

// Python sends the caller's timeout as seconds, which null leaves unset and a zero keeps for the core to read as unset
pub(crate) fn timeout(
    fields: &mut Fields,
    provider: &str,
) -> Result<Option<Duration>, ConvertError> {
    match fields.shift_remove("timeout") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => seconds(&number, provider),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(field(provider, "timeout"))
        }
    }
}

fn seconds(number: &Number, provider: &str) -> Result<Option<Duration>, ConvertError> {
    let given = number.as_f64().ok_or_else(|| field(provider, "timeout"))?;
    Duration::try_from_secs_f64(given)
        .map(Some)
        .map_err(|_source| field(provider, "timeout"))
}
