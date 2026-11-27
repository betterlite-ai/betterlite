use std::collections::BTreeMap;

use betterlite_core::api::vector_store_http_write::config::WriteParams;
use betterlite_core::api::vector_store_read::error::{
    VectorStoreReadError, VectorStoreReadFailure,
};
use betterlite_core::api::vector_store_write::error::{
    VectorStoreWriteError, VectorStoreWriteFailure,
};
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreExpirationPolicy;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde::Serialize;

pub(crate) mod requests;
mod tests;

// vector_store_retrieve's arguments: the id, the call's **kwargs and its HTTP extras
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetrieveWire {
    pub(crate) vector_store_id: String,
    #[serde(default)]
    pub(crate) kwargs: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
}

// vector_store_list's arguments: its four cursor fields, where an explicit null stays distinct from an absent default
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListWire {
    #[serde(default)]
    pub(crate) after: Option<String>,
    #[serde(default)]
    pub(crate) before: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<Option<i64>>,
    #[serde(default)]
    pub(crate) order: Option<Option<String>>,
    #[serde(default)]
    pub(crate) kwargs: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
}

// vector_store_update's arguments: the id, the three named fields and the call's **kwargs
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateWire {
    pub(crate) vector_store_id: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) expires_after: Option<VectorStoreExpirationPolicy>,
    #[serde(default)]
    pub(crate) metadata: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) kwargs: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
}

// vector_store_delete's arguments: the id and the shared transport
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteWire {
    pub(crate) vector_store_id: String,
    #[serde(default)]
    pub(crate) kwargs: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the vector store call takes no files, and {count} were attached")]
    Files { count: usize },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

// a kwargs value whose type GenericLiteLLMParams cannot hold, named where it entered
#[derive(Debug, thiserror::Error)]
#[error("{field} must be {expected}")]
pub(crate) struct FieldError {
    pub(crate) field: &'static str,
    pub(crate) expected: &'static str,
}

// the boundary every native method crosses: a wire of the shape that method takes, and no attached file
pub(crate) fn parse<'text, T>(text: &'text str, attached: usize) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    if attached != 0 {
        return Err(ConvertError::Files { count: attached });
    }
    serde_json::from_str(text).map_err(|source| ConvertError::Json { source })
}

// GenericLiteLLMParams(**kwargs): the three transport fields the handlers read, each named when its type is wrong
pub(crate) fn params(kwargs: &IndexMap<String, Json>) -> Result<WriteParams, FieldError> {
    Ok(WriteParams {
        api_base: text(kwargs, "api_base")?,
        api_key: text(kwargs, "api_key")?.map(SecretString::from),
        ssl_verify: flag(kwargs, "ssl_verify")?,
    })
}

fn text(
    kwargs: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<String>, FieldError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(FieldError {
                field,
                expected: "a string",
            })
        }
    }
}

fn flag(kwargs: &IndexMap<String, Json>, field: &'static str) -> Result<Option<bool>, FieldError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(Json::Text(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(FieldError {
                field,
                expected: "a boolean",
            })
        }
    }
}

// The JSON text a NativeVectorManageError carries: LiteLLM's exception name and the fields its class keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NativeErrorPayload {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: u16,
    pub(crate) llm_provider: String,
    pub(crate) model: String,
}

// exception_type(model=None, custom_llm_provider=<the call's provider>, original_exception=...) for one mapped class
fn payload_of(class: &ErrorClass, provider: String) -> NativeErrorPayload {
    NativeErrorPayload {
        kind: class_name(class).to_owned(),
        message: class_message(class),
        status_code: class_status(class).unwrap_or(500),
        llm_provider: provider,
        model: String::new(),
    }
}

// exception_type names the resolved provider, and an unconfigured read names the one it could not find
pub(crate) fn read_payload(
    failure: &VectorStoreReadFailure,
    requested: &str,
) -> NativeErrorPayload {
    payload_of(&failure.class, read_provider(&failure.error, requested))
}

fn read_provider(error: &VectorStoreReadError, requested: &str) -> String {
    match error {
        VectorStoreReadError::Unconfigured { provider, .. } => provider.clone(),
        VectorStoreReadError::Resolution(_)
        | VectorStoreReadError::Logging(_)
        | VectorStoreReadError::Read(_) => requested.to_owned(),
    }
}

pub(crate) fn write_payload(
    failure: &VectorStoreWriteFailure,
    requested: &str,
) -> NativeErrorPayload {
    payload_of(&failure.class, write_provider(&failure.error, requested))
}

fn write_provider(error: &VectorStoreWriteError, requested: &str) -> String {
    match error {
        VectorStoreWriteError::InvalidProvider { provider }
        | VectorStoreWriteError::Unsupported { provider, .. } => provider.clone(),
        VectorStoreWriteError::Resolution(_)
        | VectorStoreWriteError::Params(_)
        | VectorStoreWriteError::Timeout { .. }
        | VectorStoreWriteError::Logging(_)
        | VectorStoreWriteError::Write(_) => requested.to_owned(),
    }
}
