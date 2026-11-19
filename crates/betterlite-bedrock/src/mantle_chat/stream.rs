use std::sync::Arc;

use betterlite_anthropic::stream_decode::ChunkStream;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_core::api::messages_stream_logging::StreamDeps;
use betterlite_core::api::messages_stream_logging::seams::StreamSeams;
use betterlite_core::api::messages_thinking::stream::native_stream_factory;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json as ContractJson;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;

use crate::claude_platform::stream::{StreamFlags, get_model_response_iterator};

use super::error::MantleMessagesError;

// get_model_response_iterator: B581's native Anthropic SSE iterator over the streaming response, with sync_stream and json_mode unchanged.
#[must_use]
pub fn model_response_iterator<Source, Counter>(
    streaming_response: Source,
    sync_stream: bool,
    json_mode: Option<bool>,
    counter: Counter,
    ids: &dyn IdSource,
    max_event_bytes: usize,
) -> ChunkStream<Source, Counter> {
    get_model_response_iterator(
        streaming_response,
        StreamFlags {
            sync_stream,
            json_mode,
        },
        counter,
        ids,
        max_event_bytes,
    )
}

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
