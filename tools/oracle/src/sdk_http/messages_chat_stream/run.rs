use std::collections::VecDeque;
use std::mem::take;
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

// What one pull of the chosen iterator left: an event, a frame, the end, or the wrapper's failure
enum Pulled {
    Event(Value),
    Frame(Vec<u8>),
    End,
    Failure(StreamFailure),
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
    // One call of whichever __anext__ the case drives, then what it left behind
    async fn step(
        &mut self,
        step: ChatStep,
        diagnostics: &Diagnostics,
    ) -> Result<ChatStreamStepOutcome, SdkHttpError> {
        let left = self.drive(step).await?;
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
            diagnostics: take(
                &mut *diagnostics
                    .entries
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner),
            ),
            view: observed,
        })
    }

    // One pull of the chosen iterator, in the shape that side reports it
    async fn pull(&mut self) -> Result<Pulled, SdkHttpError> {
        match self {
            Self::Wrapper(stream) => match stream.next_event().await {
                Ok(Some(event)) => Ok(Pulled::Event(normalized(&event, &stream.state.message_id)?)),
                Ok(None) => Ok(Pulled::End),
                Err(error) => Ok(Pulled::Failure(error.into_stream_failure())),
            },
            Self::View { view, .. } => view
                .next_frame()
                .await
                .map_or_else(|| Ok(Pulled::End), |frame| Ok(Pulled::Frame(frame))),
        }
    }

    // One call of the case's chosen __anext__: the wrapper hands a failure over, the view writes it as one error frame
    async fn drive(&mut self, step: ChatStep) -> Result<StepEvents, SdkHttpError> {
        let mut left = StepEvents {
            events: Vec::new(),
            frames: Vec::new(),
            end: false,
            failure: None,
        };
        if step == ChatStep::Close {
            if let Self::View { view, .. } = self {
                view.aclose();
            }
            return Ok(left);
        }
        loop {
            match self.pull().await? {
                Pulled::Event(event) => left.events.push(event),
                Pulled::Frame(frame) => left
                    .frames
                    .push(String::from_utf8(frame).map_err(SdkHttpError::FrameUtf8)?),
                Pulled::End => {
                    left.end = true;
                    break;
                }
                Pulled::Failure(raised) => {
                    left.failure = Some(failure_of(&raised));
                    break;
                }
            }
            if step == ChatStep::Next {
                break;
            }
        }
        Ok(left)
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
        .map(|recorded| recorded.iter().map(|chunk| chunk.id.clone()).collect());
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
