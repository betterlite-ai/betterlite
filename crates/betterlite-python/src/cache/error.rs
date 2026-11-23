use betterlite_cache::backend::BackendError;
use betterlite_cache::cache_key::error::CacheKeyError;
use betterlite_cache::codec::CodecError;
use betterlite_cache::configuration::backend_construction::BackendConstructionError;
use betterlite_cache::configuration::cache::CacheLifecycleError;
use betterlite_cache::embedding::embedding_cache::EmbeddingCacheError;
use betterlite_cache::envelope::decode::DecodeError;
use betterlite_cache::envelope::facade::WriteError;
use betterlite_cache::handler::HandlerError;
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_redact::error::RedactionError;
use pyo3::PyErr;
use pyo3::exceptions::{PyNotImplementedError, PyRuntimeError, PyValueError};

mod tests;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CacheError {
    #[error("the cache settings are not JSON of the expected shape: {source}")]
    Settings {
        #[source]
        source: serde_json::Error,
    },
    #[error("the cache request is not JSON of the expected shape: {source}")]
    Request {
        #[source]
        source: serde_json::Error,
    },
    #[error("the cache request result is not an embedding response: {source}")]
    Result {
        #[source]
        source: serde_json::Error,
    },
    #[error("the cache request carries no result")]
    MissingResult,
    #[error("the cache request carries no input")]
    MissingInput,
    #[error("the cache type {cache_type:?} is not built in the native cache")]
    UnsupportedType { cache_type: LiteLlmCacheType },
    #[error("the cache backend could not be built")]
    Construction {
        #[source]
        source: Box<BackendConstructionError>,
    },
    #[error("the cache flush size does not fit the envelope: {size}")]
    FlushSize { size: usize },
    #[error("the cache key request is not valid: {source}")]
    Handler {
        #[from]
        source: HandlerError,
    },
    #[error("the cache key could not be derived: {source}")]
    Key {
        #[from]
        source: CacheKeyError,
    },
    #[error("the cached value could not be read: {source}")]
    Decode {
        #[from]
        source: DecodeError,
    },
    #[error("the embedding cache entry could not be built: {source}")]
    Embedding {
        #[from]
        source: EmbeddingCacheError,
    },
    #[error("the cache write failed: {source}")]
    Write {
        #[from]
        source: WriteError,
    },
    #[error("the cache lifecycle call failed: {source}")]
    Lifecycle {
        #[from]
        source: CacheLifecycleError,
    },
    #[error("the cache value could not be converted: {source}")]
    Codec {
        #[from]
        source: CodecError,
    },
    #[error("the cache log redactor could not be built: {source}")]
    Redactor {
        #[source]
        source: RedactionError,
    },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl CacheError {
    // A malformed request or an invalid key request is the caller's value error; a backend failure is runtime.
    pub(crate) const fn is_value_error(&self) -> bool {
        matches!(
            self,
            Self::Settings { .. }
                | Self::Request { .. }
                | Self::Result { .. }
                | Self::MissingResult
                | Self::MissingInput
                | Self::Handler { .. }
                | Self::Key { .. }
        )
    }

    // A backend that cannot do the operation answers Python's NotImplementedError, as LiteLLM's BaseCache stub does.
    pub(crate) fn is_not_implemented(&self) -> bool {
        let Self::Write {
            source: WriteError::Backend { source },
        } = self
        else {
            return false;
        };
        matches!(source, BackendError::Unsupported(_))
    }
}

impl From<CacheError> for PyErr {
    fn from(error: CacheError) -> Self {
        if error.is_value_error() {
            return PyValueError::new_err(error.to_string());
        }
        if error.is_not_implemented() {
            return PyNotImplementedError::new_err(error.to_string());
        }
        PyRuntimeError::new_err(error.to_string())
    }
}
