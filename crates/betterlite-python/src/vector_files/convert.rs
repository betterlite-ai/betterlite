use std::collections::BTreeMap;

use betterlite_core::api::vector_file_create::{CreateFileRequest, TransportRequest};
use betterlite_core::api::vector_file_read::error::VectorFileReadError;
use betterlite_core::api::vector_file_read::error::failure as read_failure;
use betterlite_core::api::vector_file_write::error::VectorFileWriteFailure;
use betterlite_core::api::vector_store_search::error::SdkFailure as FileSdkFailure;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_file_contracts::records::Attributes;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;

pub(super) mod list;
pub(super) mod read;
mod tests;
pub(super) mod write;

// vector_store_file_create's arguments: the two ids, the named optionals, the call's **kwargs and its HTTP extras
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateWire {
    pub(crate) vector_store_id: String,
    pub(crate) file_id: String,
    #[serde(default)]
    pub(crate) attributes: Option<Attributes>,
    #[serde(default)]
    pub(crate) chunking_strategy: Option<IndexMap<String, Json>>,
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
    #[serde(default)]
    pub(crate) kwargs: IndexMap<String, Json>,
}

// vector_store_file_list's arguments: the store id, the five query fields and the extras its handler takes
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListWire {
    pub(crate) vector_store_id: String,
    #[serde(default)]
    pub(crate) after: Option<String>,
    #[serde(default)]
    pub(crate) before: Option<String>,
    #[serde(default)]
    pub(crate) filter: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<i64>,
    #[serde(default)]
    pub(crate) order: Option<String>,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) kwargs: IndexMap<String, Json>,
}

// retrieve and retrieve_content take the same arguments, and neither handler has a body or a query to fill
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadWire {
    pub(crate) vector_store_id: String,
    pub(crate) file_id: String,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) kwargs: IndexMap<String, Json>,
}

// update and delete share their ids and extras; only update carries attributes and a body
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WriteWire {
    pub(crate) vector_store_id: String,
    pub(crate) file_id: String,
    #[serde(default)]
    pub(crate) attributes: Option<Attributes>,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) kwargs: IndexMap<String, Json>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the vector store file call takes no files, and {count} were attached")]
    Files { count: usize },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
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

// the call B1072's create reads, borrowing the wire for as long as the call runs
pub(crate) fn create_request(wire: &CreateWire) -> CreateFileRequest<'_> {
    CreateFileRequest {
        transport: TransportRequest {
            vector_store_id: &wire.vector_store_id,
            extra_headers: wire.extra_headers.as_ref(),
            extra_query: wire.extra_query.as_ref(),
            extra_body: wire.extra_body.as_ref(),
            timeout: wire.timeout,
            custom_llm_provider: wire.custom_llm_provider.as_deref(),
            kwargs: &wire.kwargs,
        },
        file_id: &wire.file_id,
        attributes: wire.attributes.as_ref(),
        chunking_strategy: wire.chunking_strategy.as_ref(),
    }
}

// The JSON text a NativeVectorFileError carries: LiteLLM's exception name and the fields its class keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NativeErrorPayload {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: u16,
    pub(crate) llm_provider: String,
    pub(crate) model: String,
}

// What a failed call raises: the provider the wrapper resolved, and the family error it came from
#[derive(Debug)]
pub(crate) struct CallError {
    pub(crate) provider: String,
    pub(crate) source: CallFailure,
}

#[derive(Debug)]
pub(crate) enum CallFailure {
    File(Box<FileSdkFailure>),
    Read(VectorFileReadError),
    Write(Box<VectorFileWriteFailure>),
}

// exception_type(model=None, custom_llm_provider=<the resolved provider>, original_exception=<the family error>)
pub(crate) fn payload_of(error: &CallError, redactor: &Redactor) -> NativeErrorPayload {
    match &error.source {
        CallFailure::File(source) => from_file(source, &error.provider),
        CallFailure::Read(source) => from_read(source, &error.provider, redactor),
        CallFailure::Write(source) => from_write(source, &error.provider),
    }
}

fn from_file(source: &FileSdkFailure, provider: &str) -> NativeErrorPayload {
    NativeErrorPayload {
        kind: source.class.clone(),
        message: source.message.clone(),
        status_code: source.status,
        llm_provider: provider.to_owned(),
        model: String::new(),
    }
}

fn from_read(
    source: &VectorFileReadError,
    provider: &str,
    redactor: &Redactor,
) -> NativeErrorPayload {
    let sdk = read_failure(source, redactor);
    NativeErrorPayload {
        kind: sdk.class,
        message: sdk.message,
        status_code: sdk.status,
        llm_provider: provider.to_owned(),
        model: String::new(),
    }
}

// A write failure names its class through raised(); a class the SDK has no exception for is APIConnectionError's 500
fn from_write(source: &VectorFileWriteFailure, provider: &str) -> NativeErrorPayload {
    let raised = match source.raised() {
        Ok(raised) => raised,
        Err(_unprojected) => {
            return NativeErrorPayload {
                kind: String::from("APIConnectionError"),
                message: source.error.to_string(),
                status_code: 500,
                llm_provider: provider.to_owned(),
                model: String::new(),
            };
        }
    };
    NativeErrorPayload {
        kind: raised.class.to_owned(),
        message: raised.message,
        status_code: raised.status,
        llm_provider: provider.to_owned(),
        model: String::new(),
    }
}
