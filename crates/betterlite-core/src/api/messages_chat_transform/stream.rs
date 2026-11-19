use std::collections::VecDeque;

use betterlite_anthropic::api::messages_errors::decode::WireDecodeLimits;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_logging::logging_state::LoggingState;
use indexmap::IndexMap;

use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_chat_stream::driver::ChatStreamDriver;
use crate::api::messages_chat_stream::sse::SyncFrames;
use crate::api::messages_chat_stream::view::AnthropicSseStream;
use crate::api::messages_context_clear::result::PolyfillResult;
use crate::api::messages_stream_split::{AsyncChunkSource, AsyncChunkSplitter, ChunkSource};
use crate::api::messages_stream_usage::state::{StreamSetup, StreamState};
use crate::api::messages_sync_stream::driver::SyncDriver;

// The keyword arguments LiteLLM's streaming factory takes, with is_async defaulting to true as it does there.
pub struct StreamOptions<'call> {
    pub model: &'call str,
    pub tool_name_mapping: Option<&'call IndexMap<String, String>>,
    pub polyfill_result: Option<&'call PolyfillResult>,
    pub logging: Option<&'call mut LoggingState>,
    pub limits: ApiLimits,
    pub is_async: bool,
}

impl<'call> StreamOptions<'call> {
    #[must_use]
    pub fn new(model: &'call str) -> Self {
        Self {
            model,
            tool_name_mapping: None,
            polyfill_result: None,
            logging: None,
            limits: ApiLimits::default(),
            is_async: true,
        }
    }
}

// The view an async caller gets or the iterator a synchronous caller gets; both are boxed so the two modes stay within 3x.
pub enum StreamOutput<S, I> {
    Async(Box<AnthropicSseStream<S, I>>),
    Sync(Box<SyncFrames<S, I>>),
}

// One wrapper state feeds either iterator; only the iteration mode decides which one the caller sees.
#[must_use]
pub fn translate_output_stream<S, I>(
    source: S,
    options: StreamOptions<'_>,
    ids: I,
) -> StreamOutput<S, I>
where
    S: AsyncChunkSource + ChunkSource + Send,
    I: IdSource,
{
    let setup = setup_of(&options);
    if options.is_async {
        let state = StreamState::new(
            options.logging,
            AsyncChunkSplitter {
                source,
                buffer: VecDeque::new(),
            },
            setup,
        );
        return StreamOutput::Async(Box::new(AnthropicSseStream::new(
            ChatStreamDriver { state, ids },
            wire_limits(options.limits),
        )));
    }
    let state = StreamState::new(options.logging, source, setup);
    StreamOutput::Sync(Box::new(SyncFrames {
        driver: SyncDriver { state, ids },
    }))
}

// The setup owns its copies, so a later edit of the polyfill result cannot reach a running stream.
fn setup_of(options: &StreamOptions<'_>) -> StreamSetup {
    let polyfill_result = options.polyfill_result;
    StreamSetup {
        model: options.model.to_owned(),
        tool_name_mapping: options.tool_name_mapping.cloned().unwrap_or_default(),
        applied_edits: polyfill_result
            .and_then(PolyfillResult::applied_edits_for_response)
            .unwrap_or_default(),
        compaction_block: polyfill_result.and_then(|result| result.compaction_block.clone()),
        iterations_usage: polyfill_result.and_then(|result| result.iterations_usage.clone()),
    }
}

fn wire_limits(limits: ApiLimits) -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: limits.max_json_bytes,
        max_event_bytes: limits.max_event_bytes,
        max_items: limits.max_items,
    }
}
