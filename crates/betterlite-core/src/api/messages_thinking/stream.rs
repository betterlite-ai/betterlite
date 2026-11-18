use std::sync::Arc;

use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::messages_stream_logging::seams::StreamSeams;
use crate::api::messages_stream_logging::{MessagesStreamLogging, StreamDeps};

// get_async_streaming_response_iterator: B2436's chunk processor gets the live response, body and logging object unchanged.
pub fn native_stream_factory<S>(
    deps: StreamDeps<S>,
    response: S::Response,
    request_body: IndexMap<String, Json>,
    logging_obj: Arc<StreamLoggingObject>,
) -> S::Stream
where
    S: StreamSeams,
{
    MessagesStreamLogging::new(deps, Arc::clone(&logging_obj), request_body.clone())
        .streaming_response_iterator(response, request_body, logging_obj)
}
