use std::future::Future;
use std::sync::Arc;

use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_logging::worker::Callback;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use thiserror::Error;

// An exception as the warnings print it: the Python type name, then its text
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{kind}({message})")]
pub struct ReportedFailure {
    pub kind: String,
    pub message: String,
}

pub type LoggingFuture = BoxFuture<'static, Result<(), ReportedFailure>>;

pub struct LoggingJob {
    pub future: LoggingFuture,
}

impl Callback for LoggingJob {
    type Error = ReportedFailure;

    fn name(&self) -> &'static str {
        "messages_stream_logging"
    }

    fn call(self) -> impl Future<Output = Result<(), ReportedFailure>> + Send + 'static {
        self.future
    }
}

// B3608's _route_streaming_logging_to_handler arguments
pub struct RouteStreamingLogging {
    pub logging_obj: Arc<StreamLoggingObject>,
    pub success_handler: Arc<PassThroughEndpointLogging>,
    pub url_route: &'static str,
    pub request_body: IndexMap<String, Json>,
    pub endpoint_type: EndpointType,
    pub start_time: DateTime<FixedOffset>,
    pub raw_bytes: Vec<Vec<u8>>,
    pub end_time: DateTime<FixedOffset>,
}

// B3609's chunk_processor arguments
pub struct ChunkProcessorCall<Response> {
    pub response: Response,
    pub request_body: IndexMap<String, Json>,
    pub logging_obj: Arc<StreamLoggingObject>,
    pub endpoint_type: EndpointType,
    pub start_time: DateTime<FixedOffset>,
    pub success_handler: Arc<PassThroughEndpointLogging>,
    pub url_route: &'static str,
}

pub trait StreamSeams: Send + Sync {
    type Response: Send;
    type Stream;

    fn route_streaming_logging(
        &self,
        call: RouteStreamingLogging,
    ) -> Result<LoggingFuture, ReportedFailure>;

    fn chunk_processor(&self, call: ChunkProcessorCall<Self::Response>) -> Self::Stream;
}
