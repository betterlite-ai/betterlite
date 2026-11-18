mod decode;
pub mod error;
mod logging;
pub mod status;
mod transport;
mod url;

pub use decode::decode_async_embedding_status;
pub use error::{EmbeddingJobsError, FailureBody};
pub use status::{
    AsyncInvokeStatusCall, StatusCredentials, StatusServices, get_async_invoke_status,
};
pub use transport::{StatusReply, StatusTransport, StatusWireRequest};
pub use url::{StatusEndpoint, async_invoke_status_url};

#[cfg(test)]
mod tests;
