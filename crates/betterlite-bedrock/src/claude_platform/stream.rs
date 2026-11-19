use betterlite_anthropic::stream_decode::ChunkStream;
use betterlite_anthropic::stream_events::StreamEventConverter;
use betterlite_anthropic::stream_state::state::{IdSource, StreamOptions, StreamState};
use indexmap::IndexMap;

// get_model_response_iterator: the shared Anthropic SSE iterator, never the AWS binary event decoder.
#[must_use]
pub fn get_model_response_iterator<Source, Counter>(
    streaming_response: Source,
    sync_stream: bool,
    json_mode: Option<bool>,
    counter: Counter,
    ids: &dyn IdSource,
    max_event_bytes: usize,
) -> ChunkStream<Source, Counter> {
    // LiteLLM stores bool(json_mode), and its default parameter reads a missing value as false.
    let options = StreamOptions {
        sync_stream,
        json_mode: Some(json_mode.unwrap_or(false)),
        speed: None,
        tool_name_reverse_map: IndexMap::new(),
    };
    ChunkStream {
        converter: StreamEventConverter {
            state: StreamState::new(options, ids),
            counter,
            json_could_close: false,
        },
        source: streaming_response,
        source_ended: false,
        accumulated_bytes: 0,
        max_event_bytes,
    }
}
