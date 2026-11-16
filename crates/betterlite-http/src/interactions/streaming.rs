use crate::interactions::base::{ErrorClasses, Logging, UpstreamResponse};
use crate::proxy::StreamResponse;

pub mod asynchronous;

pub mod synchronous;

pub(crate) mod tests;

// the iterators live in betterlite-core, which depends on this crate, so the provider config builds them here
pub trait SyncStreamingConfig: ErrorClasses {
    type SyncStream;

    fn create_sync_stream_iterator(
        &self,
        response: UpstreamResponse,
        model: Option<&str>,
        logging: &mut Logging,
    ) -> Result<Self::SyncStream, Self::Error>;
}

pub trait AsyncStreamingConfig: ErrorClasses {
    type AsyncStream;

    fn create_async_stream_iterator(
        &self,
        response: StreamResponse,
        model: Option<&str>,
        logging: &mut Logging,
    ) -> Result<Self::AsyncStream, Self::Error>;
}
