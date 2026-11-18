use super::cleanup::CleanupGuard;
use super::failure::{FailureContext, failure_frame, unix_timestamp};
use super::*;
use crate::stream_chunks::{
    ChunkKind, DONE_FRAME, MAX_RAW_SSE_BUFFER_CHARS, StreamItem,
    fallback::{client_requested_model, fallback_metadata, include_fallback_errors},
    frames::{ends_with_frame_delimiter, pop_sse_frame, sse_data_frame},
    restamp::{RestampInput, restamp_chunk_model},
    serialize::{chunk_response_text, is_usage_artifact, serialize_chunk},
};
use betterlite_models::model_registration::JsonObject;
use futures_util::StreamExt as _;
use std::sync::Arc;

pub(super) fn generate<Key: Send + Sync + 'static>(
    generator: &SseDataGenerator<Key>,
    input: GeneratorInput<StreamSource, Key>,
) -> BoxStream<'static, Result<Bytes, io::Error>> {
    let hooks = generator.hooks.clone();
    let error_state_source = generator.error_state.clone();
    let cleanup = generator.cleanup.clone();
    let router = generator.router.clone();
    let global_keepalive = generator.global_keepalive.clone();
    let expose_fallback_errors = generator.expose_fallback_errors;
    let is_model_router = generator.is_model_router;
    let body = async_stream::stream! {
    let GeneratorInput {
        response,
        key,
        request_data,
        request,
        responses_stream_errors,
    } = input;
    let requested = client_requested_model(&request_data);
    let mut fallback = fallback_metadata(&response.hidden);
    let include = include_fallback_errors(expose_fallback_errors, &request_data);
    let flags = super::setup::RequestFlags::read(&request_data);
    let raw = flags.raw;
    let strip = flags.strip;
    let skip_done = flags.skip_done;
    let wrapped = hooks.needs_iterator_wrap();
    let mut items = response.items;
    if wrapped {
        items = hooks.wrap_iterator(items, &key, &request_data);
    }
    let beats = super::setup::beats(super::setup::BeatSetup {
        items,
        router: router.clone(),
        request_data: Arc::new(request_data.clone()),
        global: global_keepalive.clone(),
        hidden: &response.hidden,
    });
    let error_state = if responses_stream_errors {
        Some(error_state_source.new_state())
    } else {
        None
    };
    let mut guard = CleanupGuard {
        cleanup: cleanup.clone(),
        outcome: Some(CleanupOutcome {
            request,
            request_data: request_data.clone(),
            key,
            response: response.snapshot,
            stream_completed: false,
            client_disconnected: false,
        }),
    };
    let mut chunk_text = String::new();
    let mut raw_buffer = String::new();
    let mut fallback_sent = false;
    let mut mismatch = false;
    let mut beats = beats;
    let mut error_message = None;
            loop {
                let Some(beat) = beats.next().await else {
                    guard.complete(true);
                    if !wrapped {
                        hooks.fire_deferred_stream_logging(&request_data);
                    }
                    if !raw_buffer.is_empty() {
                        let mut frame = std::mem::take(&mut raw_buffer);
                        if !ends_with_frame_delimiter(&frame) {
                            frame.push_str("\n\n");
                        }
                        yield Ok(Bytes::from(frame));
                    }
                    if let Some(message) = error_message.take() {
                        yield Ok(Bytes::from(message));
                    }
                    if !skip_done {
                        yield Ok(Bytes::from(DONE_FRAME));
                    }
                    break;
                };
                match beat {
                        crate::stream_keepalive::wrapper::Beat::Ping => yield Ok(Bytes::from_static(b": ping\n\n")),
                        crate::stream_keepalive::wrapper::Beat::Item(Err(failure)) => {
                        let output = FailureContext {
                            hooks: hooks.as_ref(),
                            request_data: &request_data,
                            error_state: &mut error_state,
                            guard: &mut guard,
                            skip_done,
                        }
                        .run(failure)
                        .await;
                        for frame in output { yield frame; }
                        break;
                    }
                        crate::stream_keepalive::wrapper::Beat::Item(Ok(mut item)) => {
                        if hooks.needs_per_chunk_hook() {
                            match hooks
                                .chunk_hook(
                                    item,
                                    &guard.outcome.as_ref().expect("active cleanup").key,
                                    &request_data,
                                    (!chunk_text.is_empty()).then_some(chunk_text.as_str()),
                                )
                                .await
                            {
                                Ok(returned) => item = returned,
                                Err(failure) => {
                                    let frames = FailureContext {
                                        hooks: hooks.as_ref(),
                                        request_data: &request_data,
                                        error_state: &mut error_state,
                                        guard: &mut guard,
                                        skip_done,
                                    }
                                    .run(failure)
                                    .await;
                                    for frame in frames { yield frame; }
                                    break;
                                }
                            }
                            if let StreamItem::Chunk(chunk) = &item {
                                if matches!(
                                    chunk.kind,
                                    ChunkKind::ModelStream | ChunkKind::ModelFull
                                ) {
                                    chunk_text.push_str(&chunk_response_text(chunk));
                                }
                            }
                        }
                        if fallback.model.is_none() {
                            if let Some(hidden) = item.hidden_params() {
                                let next = fallback_metadata(hidden);
                                if next.attempted {
                                    fallback.attempted = true;
                                    fallback.model = next.model;
                                    if fallback.errors.is_empty() {
                                        fallback.errors = next.errors;
                                    }
                                }
                            }
                        }
                        let pending = include
                            && fallback.attempted
                            && !fallback.errors.is_empty()
                            && !fallback_sent;
                        mismatch = restamp_chunk_model(
                            &mut item,
                            &RestampInput {
                                requested_model: &requested,
                                request_data: &request_data,
                                fallback_attempted: fallback.attempted,
                                fallback_model: fallback.model.as_deref(),
                                is_model_router,
                            },
                            mismatch,
                        );
                        if strip
                            && matches!(&item, StreamItem::Chunk(chunk) if is_usage_artifact(chunk))
                        {
                            if pending {
                                fallback_sent = true;
                                yield Ok(Bytes::from(fallback_event_frame(
                                        fallback.model.as_deref(),
                                        &fallback.errors,
                                        unix_timestamp(),
                                    )));
                                continue;
                            }
                            continue;
                        }
                        if let Some(state) = error_state.as_mut() {
                            state.observe_chunk(&item);
                        }
                        let mut raw_passthrough = false;
                        let frame = match item {
                            StreamItem::Chunk(chunk) => {
                                Some(sse_data_frame(&serialize_chunk(&chunk)))
                            }
                            StreamItem::Bytes(bytes) => {
                                let text = String::from_utf8_lossy(&bytes).into_owned();
                                if raw {
                                    raw_buffer.push_str(&text);
                                    raw_passthrough = true;
                                    None
                                } else if text.starts_with("data:")
                                    || text.starts_with("event:")
                                    || text.starts_with(':')
                                {
                                    let mut text = text;
                                    if !ends_with_frame_delimiter(&text) {
                                        text.push_str("\n\n");
                                    }
                                    Some(text)
                                } else {
                                    Some(sse_data_frame(&text))
                                }
                            }
                            StreamItem::Text(text) if raw => {
                                raw_buffer.push_str(&text);
                                raw_passthrough = true;
                                None
                            }
                            StreamItem::Text(text) if text.starts_with("data: ") => {
                                error_message = Some(text);
                                None
                            }
                            StreamItem::Text(text) => Some(sse_data_frame(&text)),
                        };
                        if raw_passthrough {
                            let mut raw_frames = Vec::new();
                            loop {
                                let (complete, rest) = pop_sse_frame(&raw_buffer);
                                raw_buffer = rest;
                                let Some(frame) = complete else { break };
                                raw_frames.push(frame);
                            }
                            let too_large = raw_buffer.chars().count()
                                > MAX_RAW_SSE_BUFFER_CHARS;
                            for frame in raw_frames {
                                yield Ok(Bytes::from(frame));
                            }
                            if too_large {
                                let failure = StreamFailure {
                                    message: "Raw SSE stream exceeded maximum buffered size without a frame delimiter".to_owned(),
                                    error_type: FailureType::Missing,
                                    param: FailureParam::Missing,
                                    status_code: None,
                                    kind: FailureKind::Other,
                                };
                                for output in (FailureContext {
                                    hooks: hooks.as_ref(),
                                    request_data: &request_data,
                                    error_state: &mut error_state,
                                    guard: &mut guard,
                                    skip_done,
                                })
                                .run(failure)
                                .await
                                {
                                    yield output;
                                }
                                break;
                            }
                            if pending {
                                fallback_sent = true;
                                yield Ok(Bytes::from(fallback_event_frame(
                                    fallback.model.as_deref(),
                                    &fallback.errors,
                                    unix_timestamp(),
                                )));
                            }
                            continue;
                        }
                        if let Some(mut frame) = frame {
                            if let Some(state) = error_state.as_mut() {
                                frame = state.mark_emitted(frame);
                            }
                            yield Ok(Bytes::from(frame));
                            if pending {
                                fallback_sent = true;
                                yield Ok(Bytes::from(fallback_event_frame(
                                    fallback.model.as_deref(),
                                    &fallback.errors,
                                    unix_timestamp(),
                                )));
                            }
                        }
                            if let Some(message) = error_message.take() {
                                guard.complete(true);
                                if !wrapped {
                                    hooks.fire_deferred_stream_logging(&request_data);
                                }
                                yield Ok(Bytes::from(message));
                                if !skip_done {
                                    yield Ok(Bytes::from(DONE_FRAME));
                                }
                                break;
                            }
                    }
                }
            }
    };
    body.boxed()
}
