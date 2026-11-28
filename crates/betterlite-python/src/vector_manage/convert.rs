use std::collections::BTreeMap;

use betterlite_core::api::vector_store_http_write::config::WriteParams;
use betterlite_core::api::vector_store_read::error::VectorStoreReadError;
use betterlite_core::api::vector_store_write::error::VectorStoreWriteError;
use betterlite_core::api::vector_store_write::{
    TransportRequest as WriteTransport, VectorStoreWriteRequest,
};
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreExpirationPolicy;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde::Serialize;

mod tests;

// the kwargs and HTTP overrides every vector store call carries; extra_query is accepted and never forwarded
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransportWire {
    #[serde(default)]
    pub(crate) kwargs: IndexMap<String, Json>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    pub(crate) timeout: Option<f64>,
    pub(crate) custom_llm_provider: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetrieveWire {
    pub(crate) vector_store_id: String,
    pub(crate) transport: TransportWire,
}

// vector_store_list's arguments: the four cursors the Python signature always sends, a null one left out of the request
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListWire {
    pub(crate) after: Option<String>,
    pub(crate) before: Option<String>,
    pub(crate) limit: Option<i64>,
    pub(crate) order: Option<String>,
    pub(crate) transport: TransportWire,
}

// the arguments update and delete share: the id and the three create fields a delete leaves unset
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WriteWire {
    pub(crate) vector_store_id: String,
    pub(crate) name: Option<String>,
    pub(crate) expires_after: Option<VectorStoreExpirationPolicy>,
    pub(crate) metadata: Option<IndexMap<String, Json>>,
    pub(crate) transport: TransportWire,
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
    #[error("api_base must be a string")]
    ApiBase,
    #[error("api_key must be a string")]
    ApiKey,
    #[error("ssl_verify must be a boolean")]
    SslVerify,
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

// GenericLiteLLMParams(**kwargs): the three transport fields the handlers read, each named when its Json type is wrong
pub(crate) fn params(kwargs: &IndexMap<String, Json>) -> Result<WriteParams, ConvertError> {
    let mut api_base = None;
    let mut api_key = None;
    let mut ssl_verify = None;
    for (field, value) in kwargs {
        match (field.as_str(), value) {
            ("api_base" | "api_key" | "ssl_verify", Json::Null) => {}
            ("api_base", Json::Text(base)) => api_base = Some(base.clone()),
            ("api_key", Json::Text(key)) => api_key = Some(SecretString::from(key.clone())),
            ("ssl_verify", Json::Bool(flag)) => ssl_verify = Some(*flag),
            ("api_base", _) => return Err(ConvertError::ApiBase),
            ("api_key", _) => return Err(ConvertError::ApiKey),
            ("ssl_verify", _) => return Err(ConvertError::SslVerify),
            _ => {}
        }
    }
    Ok(WriteParams {
        api_base,
        api_key,
        ssl_verify,
    })
}

// the transport fields every read carries, expanded into the read core's request records
macro_rules! read_transport {
    ($wire:ident, $params:expr) => {
        ReadTransport {
            extra_headers: $wire.transport.extra_headers.as_ref(),
            extra_query: $wire.transport.extra_query.as_ref(),
            extra_body: $wire.transport.extra_body.as_ref(),
            timeout: $wire.transport.timeout,
            custom_llm_provider: $wire.transport.custom_llm_provider.as_deref(),
            params: $params,
            kwargs: &$wire.transport.kwargs,
        }
    };
}

pub(crate) use read_transport;

macro_rules! write_transport {
    ($wire:ident, $params:expr) => {
        WriteTransport {
            vector_store_id: &$wire.vector_store_id,
            extra_headers: $wire.transport.extra_headers.as_ref(),
            extra_query: $wire.transport.extra_query.as_ref(),
            extra_body: $wire.transport.extra_body.as_ref(),
            timeout: $wire.transport.timeout,
            custom_llm_provider: $wire.transport.custom_llm_provider.as_deref(),
            params: $params,
            kwargs: &$wire.transport.kwargs,
        }
    };
}

// the call the write core takes for update and delete: a None field is left out and an empty metadata still travels
pub(crate) fn write_request<'wire>(
    wire: &'wire WriteWire,
    params: &'wire WriteParams,
) -> VectorStoreWriteRequest<'wire> {
    VectorStoreWriteRequest {
        transport: write_transport!(wire, params),
        name: wire.name.as_deref(),
        expires_after: wire.expires_after.as_ref(),
        metadata: wire.metadata.as_ref(),
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
pub(crate) fn payload_of(class: &ErrorClass, provider: String) -> NativeErrorPayload {
    NativeErrorPayload {
        kind: class_name(class).to_owned(),
        message: class_message(class),
        status_code: class_status(class).unwrap_or(500),
        llm_provider: provider,
        model: String::new(),
    }
}

// the provider a failure names: the one the core resolved, or the caller's declaration
pub(crate) enum ProviderError<'failure> {
    Read(&'failure VectorStoreReadError),
    Write(&'failure VectorStoreWriteError),
}

pub(crate) fn provider_name(error: &ProviderError<'_>, requested: &str) -> String {
    match error {
        ProviderError::Read(VectorStoreReadError::Unconfigured { provider, .. })
        | ProviderError::Write(
            VectorStoreWriteError::InvalidProvider { provider }
            | VectorStoreWriteError::Unsupported { provider, .. },
        ) => provider.clone(),
        ProviderError::Read(
            VectorStoreReadError::Resolution(_)
            | VectorStoreReadError::Logging(_)
            | VectorStoreReadError::Read(_),
        )
        | ProviderError::Write(
            VectorStoreWriteError::Resolution(_)
            | VectorStoreWriteError::Params(_)
            | VectorStoreWriteError::Timeout { .. }
            | VectorStoreWriteError::Logging(_)
            | VectorStoreWriteError::Write(_),
        ) => requested.to_owned(),
    }
}
