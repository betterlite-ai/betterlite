use std::collections::BTreeMap;

use betterlite_core::api::vector_search_execution::config::{Params, SearchQuery};
use betterlite_core::api::vector_store_search::error::{
    SdkFailure, SdkRaise, VectorStoreSearchError, failure,
};
use betterlite_core::api::vector_store_search::service::{CreateCall, Entry, SearchCall};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;

mod tests;

// vector_store_create's arguments: the five named fields, the call's **kwargs and its HTTP extras
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateWire {
    #[serde(default)]
    pub(crate) optional: Params,
    #[serde(default)]
    pub(crate) kwargs: Params,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) entry: Option<Entry>,
}

// vector_store_search's arguments: the two required ones, the named optionals and the private executor's absence
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchWire {
    pub(crate) vector_store_id: String,
    pub(crate) query: SearchQuery,
    #[serde(default)]
    pub(crate) optional: Params,
    #[serde(default)]
    pub(crate) kwargs: Params,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) extra_body: Option<Params>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) entry: Option<Entry>,
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

// the boundary every native method crosses: a wire of the shape that method takes, and no attached file
pub(crate) fn parse<'text, T>(text: &'text str, blobs: &[Vec<u8>]) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    if !blobs.is_empty() {
        return Err(ConvertError::Files { count: blobs.len() });
    }
    serde_json::from_str(text).map_err(|source| ConvertError::Json { source })
}

// the call B1064's create reads, borrowing the wire for as long as the call runs
pub(crate) fn create_call(wire: &CreateWire) -> CreateCall<'_> {
    CreateCall {
        optional: &wire.optional,
        kwargs: &wire.kwargs,
        extra_headers: wire.extra_headers.as_ref(),
        extra_body: wire.extra_body.as_ref(),
        timeout: wire.timeout,
        custom_llm_provider: wire.custom_llm_provider.as_deref(),
        entry: wire.entry.unwrap_or(Entry::Sync),
    }
}

// the call B1064's search reads; the Python layer refuses the private executor before any wire is built
pub(crate) fn search_call(wire: &SearchWire) -> SearchCall<'_> {
    SearchCall {
        vector_store_id: &wire.vector_store_id,
        query: &wire.query,
        optional: &wire.optional,
        kwargs: &wire.kwargs,
        extra_headers: wire.extra_headers.as_ref(),
        extra_body: wire.extra_body.as_ref(),
        timeout: wire.timeout,
        custom_llm_provider: wire.custom_llm_provider.as_deref(),
        executor: None,
        entry: wire.entry.unwrap_or(Entry::Sync),
    }
}

// The JSON text a NativeVectorSearchError carries: LiteLLM's exception name and the fields its class keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NativeErrorPayload {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: u16,
    pub(crate) llm_provider: String,
    pub(crate) model: String,
}

// What the native method raises: the SDK's exception, or the bare TypeError the executor check raises before the try block
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NativeRaise {
    Exception(NativeErrorPayload),
    TypeError(String),
}

// How LiteLLM's client decorator turns a vector store failure into what the caller sees
pub(crate) fn raise_payload(error: &VectorStoreSearchError, redactor: &Redactor) -> NativeRaise {
    match failure(error, redactor) {
        SdkRaise::TypeError(refusal) => NativeRaise::TypeError(refusal.message),
        SdkRaise::Exception(sdk) => NativeRaise::Exception(payload_of(&sdk, provider_of(error))),
    }
}

// exception_type(model=None, custom_llm_provider=<the resolved provider>, ...) keeps the provider and leaves the model unset
fn payload_of(sdk: &SdkFailure, provider: &str) -> NativeErrorPayload {
    NativeErrorPayload {
        kind: sdk.class.clone(),
        message: sdk.message.clone(),
        status_code: sdk.status,
        llm_provider: provider.to_owned(),
        model: String::new(),
    }
}

fn provider_of(error: &VectorStoreSearchError) -> &str {
    match error {
        VectorStoreSearchError::Search { provider, .. }
        | VectorStoreSearchError::Create { provider, .. }
        | VectorStoreSearchError::Unsupported { provider, .. } => provider.as_str(),
        VectorStoreSearchError::InvalidProvider { .. }
        | VectorStoreSearchError::UnresolvedProvider { .. }
        | VectorStoreSearchError::InvalidEmbeddingExecutor
        | VectorStoreSearchError::Field { .. }
        | VectorStoreSearchError::Logging(_)
        | VectorStoreSearchError::Timeout(_)
        | VectorStoreSearchError::Encode(_) => "",
    }
}
