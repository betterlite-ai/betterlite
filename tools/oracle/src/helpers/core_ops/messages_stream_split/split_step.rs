use std::collections::VecDeque;

use betterlite_core::api::messages_stream_split::errors::{
    StreamFailure, error_status_and_message,
};
use betterlite_core::api::messages_stream_split::{
    AsyncChunkSplitter, ChunkSplitter, next_split_chunk,
};
use betterlite_models::decisions::Json;
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

fn status_view(failure: &StreamFailure) -> StatusView {
    let (status, message) = error_status_and_message(failure);
    StatusView { status, message }
}

fn splitter_view(source: &ScriptedSource, collected: Collected) -> SplitterView {
    SplitterView {
        pulls: source.pulls,
        pieces: collected.pieces,
        ended: collected.ended,
        error: collected.error,
        chunks: source
            .chunks_attr
            .as_ref()
            .map(|chunks| chunks.iter().map(chunk_view).collect()),
        messages: source.messages_attr.as_deref().map(<[Json]>::to_vec),
        source: source.chunks.iter().map(chunk_view).collect(),
    }
}

fn sources(step: &SplitStep, failure: Option<&StreamFailure>) -> Vec<ScriptedSource> {
    std::iter::repeat_with(|| ScriptedSource {
        chunks: step.chunks.clone(),
        next: 0,
        failure: failure.cloned(),
        chunks_attr: step.chunks_attr.clone(),
        messages_attr: step.messages_attr.clone(),
        pulls: 0,
    })
    .take(step.splitters)
    .collect()
}

pub(super) fn run_split(step: &SplitStep) -> Result<SplitView, Box<HelperError>> {
    let failure = step.failure.as_ref().map(stream_failure).transpose()?;
    match step.mode {
        Mode::Sync => Ok(run_sync(step, sources(step, failure.as_ref()))),
        Mode::Async => run_async(step, sources(step, failure.as_ref())),
    }
}

fn run_sync(step: &SplitStep, sources: Vec<ScriptedSource>) -> SplitView {
    let mut runs: Vec<SyncRun> = sources
        .into_iter()
        .map(|source| SyncRun {
            splitter: ChunkSplitter {
                source,
                buffer: VecDeque::new(),
            },
            collected: Collected::default(),
        })
        .collect();
    for _ in 0..step.rounds {
        for run in &mut runs {
            if run.collected.ended || run.collected.error.is_some() {
                continue;
            }
            match run.splitter.next() {
                Some(Ok(piece)) => run.collected.pieces.push(chunk_view(&piece)),
                Some(Err(failure)) => run.collected.error = Some(status_view(&failure)),
                None => run.collected.ended = true,
            }
        }
    }
    let splitters = runs
        .into_iter()
        .map(|run| {
            let SyncRun {
                splitter,
                collected,
            } = run;
            splitter_view(&splitter.source, collected)
        })
        .collect();
    SplitView { splitters }
}

async fn drive_async(step: &SplitStep, sources: Vec<ScriptedSource>) -> SplitView {
    let mut runs: Vec<AsyncRun> = sources
        .into_iter()
        .map(|source| AsyncRun {
            splitter: AsyncChunkSplitter {
                source,
                buffer: VecDeque::new(),
            },
            collected: Collected::default(),
        })
        .collect();
    for _ in 0..step.rounds {
        for run in &mut runs {
            if run.collected.ended || run.collected.error.is_some() {
                continue;
            }
            match next_split_chunk(&mut run.splitter).await {
                Ok(Some(piece)) => run.collected.pieces.push(chunk_view(&piece)),
                Ok(None) => run.collected.ended = true,
                Err(failure) => run.collected.error = Some(status_view(&failure)),
            }
        }
    }
    let splitters = runs
        .into_iter()
        .map(|run| {
            let AsyncRun {
                splitter,
                collected,
            } = run;
            splitter_view(&splitter.source, collected)
        })
        .collect();
    SplitView { splitters }
}

fn run_async(
    step: &SplitStep,
    sources: Vec<ScriptedSource>,
) -> Result<SplitView, Box<HelperError>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .or_value_error()?;
    Ok(runtime.block_on(drive_async(step, sources)))
}
