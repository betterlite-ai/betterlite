use std::collections::VecDeque;
use std::sync::{Arc, PoisonError};

use betterlite_anthropic::api::messages_errors::decode::WireDecodeLimits;
use betterlite_anthropic::stream_state::state::UuidV4;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_chat_stream::driver::ChatStreamDriver;
use betterlite_core::api::messages_chat_stream::view::AnthropicSseStream;
use betterlite_core::api::messages_stream_split::AsyncChunkSplitter;
use betterlite_core::api::messages_stream_split::errors::StreamFailure;
use betterlite_core::api::messages_stream_usage::state::{StreamSetup, StreamState};
use serde_json::Value;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use super::call::{ChatInitial, ChatMode, ChatRunSpec, ChatStep, MessagesChatStreamCall};
use super::source::Prefetched;
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::messages_sync_stream::diagnostics::Diagnostics;
use crate::sdk_http::result::{
    ChatStreamFailure, ChatStreamRunOutcome, ChatStreamStepOutcome, ChatStreamView,
};
use crate::sdk_http::wrapper_stream::{NORMALIZED_ID, normalized, seed_object};

const SOURCE: &str = "betterlite_core::api::messages_chat_stream";

// The object a run drives: the wrapper's own __anext__, or the byte view over it that a case may also read
enum RunStream {
    Wrapper(ChatStreamDriver<Prefetched, UuidV4>),
    View {
        view: AnthropicSseStream<Prefetched, UuidV4>,
        observing: bool,
    },
}

// What one call of the chosen iterator left, before the run-level fields are added
struct StepEvents {
    events: Vec<Value>,
    frames: Vec<String>,
    end: bool,
    failure: Option<ChatStreamFailure>,
}

// One case run: the chosen view over one mock stream, driven step by step
pub(super) async fn run(
    call: &MessagesChatStreamCall,
    spec: &ChatRunSpec,
    payloads: VecDeque<Vec<u8>>,
) -> Result<ChatStreamRunOutcome, SdkHttpError> {
    let diagnostics = Diagnostics {
        source: SOURCE,
        entries: Arc::default(),
    };
    let _subscription = set_default(registry().with(diagnostics.clone()));
    let mut stream = stream_for(call, spec, payloads)?;
    let mut steps = Vec::with_capacity(spec.steps.len());
    for step in &spec.steps {
        steps.push(stream.step(*step, &diagnostics).await?);
    }
    Ok(ChatStreamRunOutcome { steps })
}

// The wrapper the case drives, seeded before its first call, wrapped in the view only when the case reads frames
fn stream_for(
    call: &MessagesChatStreamCall,
    spec: &ChatRunSpec,
    payloads: VecDeque<Vec<u8>>,
) -> Result<RunStream, SdkHttpError> {
    let mut driver = driver_for(call, spec, payloads);
    NORMALIZED_ID.clone_into(&mut driver.state.message_id);
    seeded(&mut driver.state, &spec.initial)?;
    if spec.mode == ChatMode::Wrapper {
        return Ok(RunStream::Wrapper(driver));
    }
    Ok(RunStream::View {
        view: AnthropicSseStream::new(driver, wire_limits()),
        observing: spec.mode == ChatMode::View,
    })
}

fn driver_for(
    call: &MessagesChatStreamCall,
    spec: &ChatRunSpec,
    payloads: VecDeque<Vec<u8>>,
) -> ChatStreamDriver<Prefetched, UuidV4> {
    let source = Prefetched {
        payloads,
        pulls: 0,
        failure: spec.failure.clone(),
        chunks: Vec::new(),
    };
    ChatStreamDriver {
        state: StreamState::new(
            None,
            AsyncChunkSplitter {
                source,
                buffer: VecDeque::new(),
            },
            StreamSetup {
                model: call.model.clone(),
                tool_name_mapping: call.tool_name_mapping.clone(),
                applied_edits: call.applied_edits.clone(),
                compaction_block: call.compaction_block.clone(),
                iterations_usage: call.iterations_usage.clone(),
            },
        ),
        ids: UuidV4,
    }
}

impl RunStream {
    // One call of the case's chosen iterator, then what it left behind
    async fn step(
        &mut self,
        step: ChatStep,
        diagnostics: &Diagnostics,
    ) -> Result<ChatStreamStepOutcome, SdkHttpError> {
        let left = match self {
            Self::Wrapper(stream) => wrapper_step(stream, step).await?,
            Self::View { view, .. } => view_step(view, step).await?,
        };
        let observed = match self {
            Self::View {
                view,
                observing: true,
            } => Some(view_metadata(view)),
            Self::View { .. } | Self::Wrapper(_) => None,
        };
        Ok(ChatStreamStepOutcome {
            events: left.events,
            frames: left.frames,
            end: left.end,
            failure: left.failure,
            pulls: self.pulls(),
            diagnostics: std::mem::take(
                &mut *diagnostics
                    .entries
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner),
            ),
            view: observed,
        })
    }

    // Every source pull the run made, the last one that ended the stream included
    #[must_use]
    const fn pulls(&self) -> usize {
        match self {
            Self::Wrapper(stream) => stream.state.completion_stream.source.source.pulls,
            Self::View { view, .. } => view.stream.state.completion_stream.source.source.pulls,
        }
    }
}

// One call of the wrapper's own __anext__, with the failure handed to the caller as LiteLLM lets it reach one
async fn wrapper_step(
    stream: &mut ChatStreamDriver<Prefetched, UuidV4>,
    step: ChatStep,
) -> Result<StepEvents, SdkHttpError> {
    let mut events = Vec::new();
    let mut failure = None;
    let mut end = false;
    match step {
        ChatStep::Next => match stream.next_event().await {
            Ok(Some(event)) => events.push(normalized(&event, &stream.state.message_id)?),
            Ok(None) => end = true,
            Err(error) => failure = Some(failure_of(&error.into_stream_failure())),
        },
        ChatStep::Drain => loop {
            match stream.next_event().await {
                Ok(Some(event)) => events.push(normalized(&event, &stream.state.message_id)?),
                Ok(None) => {
                    end = true;
                    break;
                }
                Err(error) => {
                    failure = Some(failure_of(&error.into_stream_failure()));
                    break;
                }
            }
        },
        ChatStep::Close => {}
    }
    Ok(StepEvents {
        events,
        frames: Vec::new(),
        end,
        failure,
    })
}

// One call of the byte view's __anext__, where a failure arrives as the one error frame instead of an error
async fn view_step(
    view: &mut AnthropicSseStream<Prefetched, UuidV4>,
    step: ChatStep,
) -> Result<StepEvents, SdkHttpError> {
    let mut frames = Vec::new();
    let mut end = false;
    match step {
        ChatStep::Next => match view.next_frame().await {
            Some(frame) => frames.push(String::from_utf8(frame).map_err(SdkHttpError::FrameUtf8)?),
            None => end = true,
        },
        ChatStep::Drain => {
            while let Some(frame) = view.next_frame().await {
                frames.push(String::from_utf8(frame).map_err(SdkHttpError::FrameUtf8)?);
            }
            end = true;
        }
        ChatStep::Close => view.aclose(),
    }
    Ok(StepEvents {
        events: Vec::new(),
        frames,
        end,
        failure: None,
    })
}

// The held buffers only exist through a seed, applied before the first call on either side
fn seeded<S>(state: &mut StreamState<S>, initial: &ChatInitial) -> Result<(), SdkHttpError> {
    state.holding_chunk = seed_object(initial.held.as_ref())?;
    state.holding_stop_reason_chunk = seed_object(initial.held_stop_reason.as_ref())?;
    state.queued_usage_chunk = initial.queued_usage;
    Ok(())
}

// What a caller of __anext__ sees: a reported status and message, or the bare exception's text
fn failure_of(failure: &StreamFailure) -> ChatStreamFailure {
    match failure {
        StreamFailure::Reported {
            status_code,
            message,
        } => ChatStreamFailure {
            kind: "reported".to_owned(),
            status_code: Some(*status_code),
            message: message.clone(),
        },
        StreamFailure::Unclassified { text } => ChatStreamFailure {
            kind: "unclassified".to_owned(),
            status_code: None,
            message: text.clone(),
        },
    }
}

// The wrapper metadata the proxy reads for disconnect-time billing
fn view_metadata(view: &AnthropicSseStream<Prefetched, UuidV4>) -> ChatStreamView {
    let chunks = view
        .chunks()
        .map(|chunks| chunks.iter().map(|chunk| chunk.id.clone()).collect());
    ChatStreamView {
        model: view.model().to_owned(),
        chunks,
    }
}

// The proxy's own wire bounds, so an oversized error frame fails the same way here
fn wire_limits() -> WireDecodeLimits {
    let limits = ApiLimits::default();
    WireDecodeLimits {
        max_json_bytes: limits.max_json_bytes,
        max_event_bytes: limits.max_event_bytes,
        max_items: limits.max_items,
    }
}
