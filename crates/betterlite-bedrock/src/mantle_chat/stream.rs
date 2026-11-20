use std::sync::Arc;

use betterlite_core::api::messages_stream_logging::StreamDeps;
use betterlite_core::api::messages_stream_logging::seams::StreamSeams;
use betterlite_core::api::messages_thinking::stream::native_stream_factory;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json as ContractJson;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;

use super::error::MantleMessagesError;

// get_async_streaming_response_iterator: B962's native iterator, with response, request body and logging unchanged.
pub fn messages_iterator<S>(
    deps: StreamDeps<S>,
    response: S::Response,
    request_body: &Object,
    logging_obj: Arc<StreamLoggingObject>,
) -> Result<S::Stream, MantleMessagesError>
where
    S: StreamSeams,
{
    let body: IndexMap<String, ContractJson> =
        serde_json::from_str(&serde_json::to_string(request_body)?)?;
    Ok(native_stream_factory(deps, response, body, logging_obj))
}
