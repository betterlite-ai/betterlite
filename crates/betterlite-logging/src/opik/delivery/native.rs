use std::sync::Arc;

use super::record::{SpanRecord, TraceRecord};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct NativeClientError {
    pub message: String,
}

// litellm reaches opik's client through `opik.api_objects.opik_client`, whose methods raise when the SDK fails.
pub trait NativeOpikClient: Send + Sync {
    fn trace(&self, trace: &TraceRecord) -> Result<(), NativeClientError>;
    fn span(&self, span: &SpanRecord) -> Result<(), NativeClientError>;
}

// `opik_client.get_client_cached()`: the SDK caches one process-wide client and returns None when opik is not installed.
pub trait NativeClientSource: Send + Sync {
    fn get_client_cached(&self) -> Option<Arc<dyn NativeOpikClient>>;
}

#[derive(Debug, Default)]
pub struct NoNativeClient;

impl NativeClientSource for NoNativeClient {
    fn get_client_cached(&self) -> Option<Arc<dyn NativeOpikClient>> {
        None
    }
}
