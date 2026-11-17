use std::collections::VecDeque;
use std::sync::{Arc, PoisonError};

use betterlite_anthropic::stream_state::state::UuidV4;
use betterlite_core::api::messages_stream_usage::state::{BlockKind, StreamSetup, StreamState};
use betterlite_core::api::messages_sync_stream::driver::SyncDriver;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use super::diagnostics::Diagnostics;
use super::source::Prefetched;
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::messages_stream_logging::outcome::Flag;
use crate::sdk_http::messages_sync_stream::call::{
    MessagesSyncStreamCall, SyncInitial, SyncRunSpec, SyncStep,
};
use crate::sdk_http::result::{SyncRunOutcome, SyncState, SyncStepOutcome};
use crate::sdk_http::wrapper_stream::{normalized, seed_object};

const SOURCE: &str = "betterlite_core::api::messages_sync_stream";

pub(super) fn run(
    call: &MessagesSyncStreamCall,
    spec: &SyncRunSpec,
    payloads: VecDeque<Vec<u8>>,
) -> Result<SyncRunOutcome, SdkHttpError> {
    let diagnostics = Diagnostics {
        source: SOURCE,
        entries: Arc::default(),
    };
    let _subscription = set_default(registry().with(diagnostics.clone()));
    let mut stream = SyncDriver {
        state: StreamState::new(
            None,
            Prefetched { payloads, pulls: 0 },
            StreamSetup {
                model: call.model.clone(),
                tool_name_mapping: call.tool_name_mapping.clone(),
                applied_edits: call.applied_edits.clone(),
                compaction_block: call.compaction_block.clone(),
                iterations_usage: call.iterations_usage.clone(),
            },
        ),
        ids: UuidV4,
    };
    seeded(&mut stream.state, &spec.initial)?;
    let mut steps = Vec::with_capacity(spec.steps.len());
    for step in &spec.steps {
        steps.push(run_step(&mut stream, *step, &diagnostics)?);
    }
    Ok(SyncRunOutcome { steps })
}

// One case step: the driver calls it asks for, then the state and diagnostics it left behind
fn run_step(
    stream: &mut SyncDriver<Prefetched, UuidV4>,
    step: SyncStep,
    diagnostics: &Diagnostics,
) -> Result<SyncStepOutcome, SdkHttpError> {
    let mut events = Vec::new();
    let end = match step {
        SyncStep::Next => match stream.next_event() {
            Some(event) => {
                events.push(normalized(&event, &stream.state.message_id)?);
                false
            }
            None => true,
        },
        SyncStep::Drain => {
            while let Some(event) = stream.next_event() {
                events.push(normalized(&event, &stream.state.message_id)?);
            }
            true
        }
    };
    Ok(SyncStepOutcome {
        events,
        end,
        state: observed(&stream.state),
        diagnostics: std::mem::take(
            &mut *diagnostics
                .entries
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        ),
    })
}

// The dormant held buffers only exist through a seed, applied before the first call on either side
fn seeded<S>(state: &mut StreamState<S>, initial: &SyncInitial) -> Result<(), SdkHttpError> {
    state.holding_chunk = seed_object(initial.held.as_ref())?;
    state.holding_stop_reason_chunk = seed_object(initial.held_stop_reason.as_ref())?;
    state.queued_usage_chunk = initial.queued_usage;
    Ok(())
}

fn observed(state: &StreamState<Prefetched>) -> SyncState {
    SyncState {
        pulls: state.completion_stream.source.pulls,
        index: state.current_content_block_index,
        block_type: block_type(state.current_content_block_type).to_owned(),
        sent_first_chunk: Flag(state.message_progress.sent_first_chunk),
        sent_content_block_start: Flag(state.block_progress.sent_content_block_start),
        sent_content_block_finish: Flag(state.block_progress.sent_content_block_finish),
        sent_last_message: Flag(state.message_progress.sent_last_message),
        queued_usage_chunk: Flag(state.queued_usage_chunk),
        holding_chunk: Flag(state.holding_chunk.is_some()),
        holding_stop_reason: Flag(state.holding_stop_reason_chunk.is_some()),
        sent_compaction_block: Flag(state.sent_compaction_block),
        sent_compaction_block_start: Flag(state.compaction_progress.sent_compaction_block_start),
        sent_compaction_block_delta: Flag(state.compaction_progress.sent_compaction_block_delta),
    }
}

const fn block_type(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Text => "text",
        BlockKind::ToolUse => "tool_use",
        BlockKind::Thinking => "thinking",
    }
}
