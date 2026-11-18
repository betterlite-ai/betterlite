use std::io;
use std::sync::Arc;

use bytes::Bytes;
use encoding_rs::UTF_8;

use super::failure::unix_timestamp;
use super::stream::{Drive, Output};
use super::types::{FailureKind, FailureParam, Flag, StreamFailure};
use crate::stream_chunks::StreamItem;
use crate::stream_chunks::fallback::{fallback_event_frame, fallback_metadata};
use crate::stream_chunks::frames::{ends_with_frame_delimiter, pop_sse_frame, sse_data_frame};
use crate::stream_chunks::restamp::{RestampInput, restamp_chunk_model};
use crate::stream_chunks::serialize::{chunk_response_text, is_usage_artifact, serialize_chunk};
use crate::stream_chunks::{ChunkKind, MAX_RAW_SSE_BUFFER_CHARS};

const RELAY_PREFIXES: [&str; 3] = ["data:", "event:", ":"];

pub(super) async fn handle_item<Key>(
    drive: &mut Drive<Key>,
    mut item: StreamItem,
) -> Vec<Result<Bytes, io::Error>>
where
    Key: Send + Sync + 'static,
{
    if drive.session.setup.per_chunk.0 {
        item = match hook(drive, item).await {
            Ok(hooked) => {
                grow_text(drive, &hooked);
                hooked
            }
            Err(failure) => return super::finish::handle_failure(drive, failure).await,
        };
    }
    latch_fallback(drive, &item);
    let pending = pending(drive);
    restamp(drive, &mut item);
    if strip_usage(drive, &item) {
        return skipped(drive, pending);
    }
    if let Some(state) = drive.session.error_state.as_mut() {
        state.observe_chunk(&item);
    }
    output(drive, item, pending)
}

async fn hook<Key>(drive: &mut Drive<Key>, item: StreamItem) -> Result<StreamItem, StreamFailure>
where
    Key: Send + Sync + 'static,
{
    let Some(key) = drive.session.outcome.as_ref().map(|outcome| &outcome.key) else {
        return Ok(item);
    };
    let so_far = (!drive.chunk_text.is_empty()).then_some(drive.chunk_text.as_str());
    let hooks = Arc::clone(&drive.session.hooks);
    hooks
        .chunk_hook(item, key, drive.session.request_data.as_ref(), so_far)
        .await
}

fn grow_text<Key>(drive: &mut Drive<Key>, item: &StreamItem)
where
    Key: Sync,
{
    if let StreamItem::Chunk(chunk) = item
        && matches!(chunk.kind, ChunkKind::ModelStream | ChunkKind::ModelFull)
    {
        let text = chunk_response_text(chunk);
        drive.chunk_text.push_str(&text);
    }
}

fn latch_fallback<Key>(drive: &mut Drive<Key>, item: &StreamItem)
where
    Key: Sync,
{
    if drive.session.setup.fallback.model.is_some() {
        return;
    }
    let Some(hidden) = item.hidden_params() else {
        return;
    };
    let next = fallback_metadata(hidden);
    if !next.attempted {
        return;
    }
    let fallback = &mut drive.session.setup.fallback;
    fallback.attempted = true;
    fallback.model = next.model;
    if fallback.errors.is_empty() {
        fallback.errors = next.errors;
    }
}

fn pending<Key>(drive: &Drive<Key>) -> bool
where
    Key: Sync,
{
    let setup = &drive.session.setup;
    setup.include.0
        && setup.fallback.attempted
        && !setup.fallback.errors.is_empty()
        && !drive.fallback_sent.0
}

fn restamp<Key>(drive: &mut Drive<Key>, item: &mut StreamItem)
where
    Key: Sync,
{
    let setup = &drive.session.setup;
    let input = RestampInput {
        requested_model: &setup.requested,
        request_data: drive.session.request_data.as_ref(),
        fallback_attempted: setup.fallback.attempted,
        fallback_model: setup.fallback.model.as_deref(),
        is_model_router: setup.is_model_router,
    };
    drive.mismatch = Flag(restamp_chunk_model(item, &input, drive.mismatch.0));
}

fn strip_usage<Key>(drive: &Drive<Key>, item: &StreamItem) -> bool
where
    Key: Sync,
{
    if !drive.session.setup.strip.0 {
        return false;
    }
    matches!(item, StreamItem::Chunk(chunk) if is_usage_artifact(chunk))
}

fn skipped<Key>(drive: &mut Drive<Key>, pending: bool) -> Vec<Result<Bytes, io::Error>>
where
    Key: Sync,
{
    if !pending {
        return Vec::new();
    }
    drive.fallback_sent = Flag(true);
    vec![Ok(Bytes::from(fallback_event(drive)))]
}

fn fallback_event<Key>(drive: &Drive<Key>) -> String
where
    Key: Sync,
{
    let model = drive.session.setup.fallback.model.as_deref();
    let created = unix_timestamp();
    fallback_event_frame(model, &drive.session.setup.fallback.errors, created)
}

fn output<Key>(
    drive: &mut Drive<Key>,
    item: StreamItem,
    pending: bool,
) -> Vec<Result<Bytes, io::Error>>
where
    Key: Send + Sync + 'static,
{
    match classify(drive, item) {
        Output::Item(frame) => {
            let marked = if let Some(state) = drive.session.error_state.as_mut() {
                state.mark_emitted(frame)
            } else {
                frame
            };
            raw_frames(drive, vec![marked], pending)
        }
        Output::Raw(frames) => raw_frames(drive, frames, pending),
        Output::Overflow(frames) => {
            drive.pending_failure = Some(StreamFailure {
                message: "Raw SSE stream exceeded maximum buffered size without a frame delimiter"
                    .to_owned(),
                error_type: None,
                param: FailureParam::Missing,
                status_code: None,
                kind: FailureKind::Other,
            });
            raw_frames(drive, frames, false)
        }
        Output::End => super::finish::end(drive),
    }
}

fn classify<Key>(drive: &mut Drive<Key>, item: StreamItem) -> Output
where
    Key: Sync,
{
    match item {
        StreamItem::Chunk(chunk) => Output::Item(sse_data_frame(&serialize_chunk(&chunk))),
        StreamItem::Bytes(bytes) => classify_bytes(drive, &bytes),
        StreamItem::Text(text) => classify_text(drive, text),
    }
}

fn classify_bytes<Key>(drive: &mut Drive<Key>, bytes: &[u8]) -> Output
where
    Key: Sync,
{
    let text = UTF_8.decode_without_bom_handling(bytes).0.into_owned();
    if drive.session.setup.raw.0 {
        return buffer_raw(drive, &text);
    }
    if RELAY_PREFIXES
        .iter()
        .any(|prefix| text.starts_with(*prefix))
    {
        let frame = if ends_with_frame_delimiter(&text) {
            text
        } else {
            format!("{text}\n\n")
        };
        return Output::Raw(vec![frame]);
    }
    Output::Item(sse_data_frame(&text))
}

fn classify_text<Key>(drive: &mut Drive<Key>, text: String) -> Output
where
    Key: Sync,
{
    if drive.session.setup.raw.0 {
        return buffer_raw(drive, &text);
    }
    if text.starts_with("data: ") {
        drive.error_message = Some(text);
        return Output::End;
    }
    Output::Item(sse_data_frame(&text))
}

fn buffer_raw<Key>(drive: &mut Drive<Key>, text: &str) -> Output
where
    Key: Sync,
{
    drive.raw_buffer.push_str(text);
    let mut frames = Vec::new();
    while let (Some(frame), rest) = pop_sse_frame(&drive.raw_buffer) {
        frames.push(frame);
        drive.raw_buffer = rest;
    }
    if drive.raw_buffer.chars().count() > MAX_RAW_SSE_BUFFER_CHARS {
        return Output::Overflow(frames);
    }
    Output::Raw(frames)
}

fn raw_frames<Key>(
    drive: &mut Drive<Key>,
    frames: Vec<String>,
    pending: bool,
) -> Vec<Result<Bytes, io::Error>>
where
    Key: Sync,
{
    let mut output = frames
        .into_iter()
        .map(|frame| Ok(Bytes::from(frame)))
        .collect::<Vec<_>>();
    if pending {
        drive.fallback_sent = Flag(true);
        output.push(Ok(Bytes::from(fallback_event(drive))));
    }
    output
}
