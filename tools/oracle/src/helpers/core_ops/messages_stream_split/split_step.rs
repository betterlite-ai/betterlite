use std::collections::VecDeque;

use betterlite_core::api::messages_stream_split::errors::{
    StreamFailure, error_status_and_message,
};
use betterlite_core::api::messages_stream_split::{
    AsyncChunkSource as _, AsyncChunkSplitter, ChunkSource as _, ChunkSplitter,
};
use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use serde::Serialize;

use super::projection::{ChunkView, StatusView, chunk_view};
use super::source::ScriptedSource;
use super::{Mode, SplitStep, stream_failure};
use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Serialize)]
pub(super) struct SplitView {
    splitters: Vec<SplitterView>,
}

#[derive(Debug, Serialize)]
struct SplitterView {
    pulls: usize,
    pieces: Vec<ChunkView>,
    ended: bool,
    error: Option<StatusView>,
    chunks: Option<Vec<ChunkView>>,
    messages: Option<Vec<Json>>,
    source: Vec<ChunkView>,
}

#[derive(Debug, Default)]
struct Collected {
    pieces: Vec<ChunkView>,
    ended: bool,
    error: Option<StatusView>,
}

struct SyncRun {
    splitter: ChunkSplitter<ScriptedSource>,
    collected: Collected,
}

struct AsyncRun {
    splitter: AsyncChunkSplitter<ScriptedSource>,
    collected: Collected,
}

// one instance iterates in the mode the case names, and a finished instance is never pulled again
enum Run {
    Sync(SyncRun),
    Async(AsyncRun),
}

impl Run {
    const fn done(&self) -> bool {
        match self {
            Self::Sync(run) => run.collected.ended || run.collected.error.is_some(),
            Self::Async(run) => run.collected.ended || run.collected.error.is_some(),
        }
    }

    fn into_view(self) -> SplitterView {
        match self {
            Self::Sync(run) => splitter_view(
                run.splitter.chunks(),
                run.splitter.messages(),
                &run.splitter.source,
                run.collected,
            ),
            Self::Async(run) => splitter_view(
                run.splitter.chunks(),
                run.splitter.messages(),
                &run.splitter.source,
                run.collected,
            ),
        }
    }
}

fn status_view(failure: &StreamFailure) -> StatusView {
    let (status, message) = error_status_and_message(failure);
    StatusView { status, message }
}

fn splitter_view(
    chunks_attr: Option<&[StreamChunk]>,
    messages_attr: Option<&[Json]>,
    source: &ScriptedSource,
    collected: Collected,
) -> SplitterView {
    SplitterView {
        pulls: source.pulls,
        pieces: collected.pieces,
        ended: collected.ended,
        error: collected.error,
        chunks: chunks_attr.map(|chunks| chunks.iter().map(chunk_view).collect()),
        messages: messages_attr.map(<[Json]>::to_vec),
        source: source.chunks.iter().map(chunk_view).collect(),
    }
}

fn runs(step: &SplitStep, failure: Option<&StreamFailure>) -> Vec<Run> {
    let template = ScriptedSource {
        chunks: step.chunks.clone(),
        next: 0,
        failure: failure.cloned(),
        chunks_attr: step.chunks_attr.clone(),
        messages_attr: step.messages_attr.clone(),
        pulls: 0,
    };
    let sources: Vec<ScriptedSource> = std::iter::repeat_n(template, step.splitters).collect();
    match step.mode {
        Mode::Sync => sources
            .into_iter()
            .map(|source| {
                Run::Sync(SyncRun {
                    splitter: ChunkSplitter {
                        source,
                        buffer: VecDeque::new(),
                    },
                    collected: Collected::default(),
                })
            })
            .collect(),
        Mode::Async => sources
            .into_iter()
            .map(|source| {
                Run::Async(AsyncRun {
                    splitter: AsyncChunkSplitter {
                        source,
                        buffer: VecDeque::new(),
                    },
                    collected: Collected::default(),
                })
            })
            .collect(),
    }
}

async fn drive(step: &SplitStep, failure: Option<&StreamFailure>) -> SplitView {
    let mut runs = runs(step, failure);
    for _ in 0..step.rounds {
        for instance in &mut runs {
            if instance.done() {
                continue;
            }
            match instance {
                Run::Sync(state) => match state.splitter.next() {
                    Some(Ok(piece)) => state.collected.pieces.push(chunk_view(&piece)),
                    Some(Err(error)) => state.collected.error = Some(status_view(&error)),
                    None => state.collected.ended = true,
                },
                Run::Async(state) => match state.splitter.next_chunk().await {
                    Ok(Some(piece)) => state.collected.pieces.push(chunk_view(&piece)),
                    Ok(None) => state.collected.ended = true,
                    Err(error) => state.collected.error = Some(status_view(&error)),
                },
            }
        }
    }
    SplitView {
        splitters: runs.into_iter().map(Run::into_view).collect(),
    }
}

pub(super) fn run_split(step: &SplitStep) -> Result<SplitView, Box<HelperError>> {
    let failure = step.failure.as_ref().map(stream_failure).transpose()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .or_value_error()?;
    Ok(runtime.block_on(drive(step, failure.as_ref())))
}
