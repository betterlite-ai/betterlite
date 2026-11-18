use std::io;

use bytes::Bytes;
use encoding_rs::CoderResult;
use futures_util::future::BoxFuture;

use super::failure::unix_timestamp;
use super::stream::{Drive, Output};
use super::types::{FailureKind, FailureParam, FailureType, Flag, StreamFailure};
use crate::stream_chunks::MAX_RAW_SSE_BUFFER_CHARS;
use crate::stream_chunks::StreamItem;
use crate::stream_chunks::fallback::{fallback_event_frame, fallback_metadata};
use crate::stream_chunks::frames::{ends_with_frame_delimiter, pop_sse_frame, sse_data_frame};
use crate::stream_chunks::restamp::{RestampInput, restamp_chunk_model};
use crate::stream_chunks::serialize::{chunk_response_text, is_usage_artifact, serialize_chunk};

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
                if let StreamItem::Chunk(chunk) = &hooked {
                    drive.chunk_text.push_str(&chunk_response_text(chunk));
                }
                hooked
            }
            Err(failure) => return super::finish::handle_failure(drive, failure).await,
        };
    }
    latch_fallback(drive, &item);
    let pending = pending(drive);
    restamp(drive, &mut item);
    if strip_usage(drive, &item) {
        return output(drive, Output::Raw(Vec::new()), pending);
    }
    if let Some(state) = drive.session.error_state.as_mut() {
        state.observe_chunk(&item);
    }
    let classified = classify(drive, item);
    output(drive, classified, pending)
}

fn hook<Key>(
    drive: &Drive<Key>,
    item: StreamItem,
) -> BoxFuture<'_, Result<StreamItem, StreamFailure>>
where
    Key: Send + Sync + 'static,
{
    let Some(key) = drive.session.outcome.as_ref().map(|outcome| &outcome.key) else {
        return Box::pin(async move { Ok(item) });
    };
    let so_far = (!drive.chunk_text.is_empty()).then_some(drive.chunk_text.as_str());
    drive
        .session
        .hooks
        .chunk_hook(item, key, drive.session.request_data.as_ref(), so_far)
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

const fn pending<Key>(drive: &Drive<Key>) -> bool
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
    classified: Output,
    mut pending: bool,
) -> Vec<Result<Bytes, io::Error>>
where
    Key: Send + Sync + 'static,
{
    let mut frames = match classified {
        Output::Item(frame) => vec![if let Some(state) = drive.session.error_state.as_mut() {
            state.mark_emitted(frame)
        } else {
            frame
        }],
        Output::Raw(frames) => frames,
        Output::Overflow(frames) => {
            drive.pending_failure = Some(StreamFailure {
                message: "Raw SSE stream exceeded maximum buffered size without a frame delimiter"
                    .to_owned(),
                error_type: FailureType::Missing,
                param: FailureParam::Missing,
                status_code: None,
                code: None,
                body: None,
                kind: FailureKind::Other,
            });
            pending = false;
            frames
        }
        Output::End => return super::finish::end(drive),
        Output::Held => return Vec::new(),
    };
    if pending {
        drive.fallback_sent = Flag(true);
        frames.push(fallback_event(drive));
    }
    frames
        .into_iter()
        .map(|frame| Ok(Bytes::from(frame)))
        .collect()
}

fn classify<Key>(drive: &mut Drive<Key>, item: StreamItem) -> Output
where
    Key: Sync,
{
    match item {
        StreamItem::Chunk(chunk) => Output::Item(sse_data_frame(&serialize_chunk(&chunk))),
        StreamItem::Bytes(bytes) => bytes_output(drive, &bytes, false),
        StreamItem::Text(text) => text_output(drive, text, TextSource::Text),
    }
}

#[derive(Clone, Copy)]
enum TextSource {
    Bytes,
    Text,
}

fn text_output<Key>(drive: &mut Drive<Key>, text: String, source: TextSource) -> Output
where
    Key: Sync,
{
    if drive.session.setup.raw.0 {
        return buffer_raw(drive, &text);
    }
    match source {
        TextSource::Bytes
            if RELAY_PREFIXES
                .iter()
                .any(|prefix| text.starts_with(*prefix)) =>
        {
            let frame = if ends_with_frame_delimiter(&text) {
                text
            } else {
                format!("{text}\n\n")
            };
            Output::Raw(vec![frame])
        }
        TextSource::Text if text.starts_with("data: ") => {
            drive.error_message = Some(text);
            Output::End
        }
        TextSource::Bytes | TextSource::Text => Output::Item(sse_data_frame(&text)),
    }
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

pub(super) fn end_source<Key>(drive: &mut Drive<Key>) -> Vec<Result<Bytes, io::Error>>
where
    Key: Send + Sync + 'static,
{
    let classified = bytes_output(drive, &[], true);
    let pending = pending(drive);
    let mut frames = output(drive, classified, pending);
    if drive.pending_failure.is_none() {
        frames.extend(super::finish::end(drive));
    }
    frames
}

fn bytes_output<Key>(drive: &mut Drive<Key>, mut bytes: &[u8], last: bool) -> Output
where
    Key: Sync,
{
    let mut text = String::with_capacity(bytes.len().max(4));
    loop {
        let (result, read, _) = drive.decoder.decode_to_string(bytes, &mut text, last);
        bytes = bytes.get(read..).unwrap_or_default();
        if result == CoderResult::InputEmpty {
            break;
        }
        text.reserve(bytes.len().max(4));
    }
    if text.is_empty() {
        Output::Held
    } else {
        text_output(drive, text, TextSource::Bytes)
    }
}
