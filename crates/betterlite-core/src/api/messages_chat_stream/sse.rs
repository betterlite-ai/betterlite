use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_models::decisions;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;

use crate::api::messages_stream_logging::{SseChunk, convert_chunk_to_sse_format};
use crate::api::messages_stream_split::ChunkSource;
use crate::api::messages_sync_stream::driver::SyncDriver;

use super::error::ChatStreamEncodeError;

// The wrapper's queue is untyped in LiteLLM, so one item is either a translated event or a chunk already in bytes
pub enum StreamItem {
    Event(Object),
    Bytes(Vec<u8>),
}

// anthropic_sse_wrapper's one chunk: the event line, then the data line exactly as Python's json.dumps writes it
pub fn encode_item(item: StreamItem) -> Result<Vec<u8>, ChatStreamEncodeError> {
    match item {
        StreamItem::Bytes(bytes) => Ok(bytes),
        StreamItem::Event(event) => {
            // the text keeps the event's key order, which Python's json.dumps writes as it was built
            let text = serde_json::to_string(&event)?;
            let mapping = serde_json::from_str::<IndexMap<String, decisions::Json>>(&text)?;
            Ok(convert_chunk_to_sse_format(SseChunk::Mapping(mapping))?)
        }
    }
}

// anthropic_sse_wrapper over B2434's synchronous driver, which ends the iteration on a failure instead of yielding it
pub struct SyncFrames<S, I> {
    pub driver: SyncDriver<S, I>,
}

impl<S, I> Iterator for SyncFrames<S, I>
where
    S: ChunkSource,
    I: IdSource,
{
    type Item = Result<Vec<u8>, ChatStreamEncodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        let event = self.driver.next_event()?;
        Some(encode_item(StreamItem::Event(event)))
    }
}
