use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::request::Parts;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use bytes::Bytes;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};

use crate::data_generator_select::{AsyncDataGenerator, GeneratorInput};
use crate::keepalive_interval::PingSetting;
use crate::stream_chunks::fallback::{
    FallbackMeta, client_requested_model, fallback_event_frame, fallback_metadata,
    include_fallback_errors,
};
use crate::stream_chunks::frames::{ends_with_frame_delimiter, pop_sse_frame, sse_data_frame};
use crate::stream_chunks::restamp::{RestampInput, RouterCheck, restamp_chunk_model};
use crate::stream_chunks::serialize::{chunk_response_text, is_usage_artifact, serialize_chunk};
use crate::stream_chunks::{
    ChunkKind, DONE_FRAME, MAX_RAW_SSE_BUFFER_CHARS, StreamChunk, StreamItem,
};
use crate::stream_keepalive::KeepaliveRouter;
use crate::stream_keepalive::resolver::KeepaliveResolver;
use crate::stream_keepalive::wrapper::{Beat, KeepaliveSource, with_keepalive};

pub type ItemStream = BoxStream<'static, Result<StreamItem, StreamFailure>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureParam {
    Missing,
    Null,
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Http,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamFailure {
    pub message: String,
    pub error_type: Option<String>,
    pub param: FailureParam,
    pub status_code: Option<u16>,
    pub kind: FailureKind,
}

pub struct StreamSource {
    pub items: ItemStream,
    pub hidden: JsonObject,
    pub snapshot: Option<Arc<dyn StreamSnapshot>>,
}

pub trait StreamSnapshot: Send + Sync {
    fn collected(&self) -> CollectedStream;
}

#[derive(Debug, Clone, Default)]
pub struct CollectedStream {
    pub chunks: Vec<JsonObject>,
    pub messages: Option<Vec<Json>>,
    pub model: Option<String>,
}

#[async_trait]
pub trait StreamHooks<Key>: Send + Sync {
    fn needs_iterator_wrap(&self) -> bool;
    fn needs_per_chunk_hook(&self) -> bool;
    fn wrap_iterator(&self, items: ItemStream, key: &Key, request_data: &JsonObject) -> ItemStream;
    async fn chunk_hook(
        &self,
        item: StreamItem,
        key: &Key,
        request_data: &JsonObject,
        str_so_far: Option<&str>,
    ) -> Result<StreamItem, StreamFailure>;
    async fn failure_hook(
        &self,
        key: &Key,
        failure: &StreamFailure,
        request_data: &JsonObject,
    ) -> Result<(), StreamFailure>;
    fn fire_deferred_stream_logging(&self, request_data: &JsonObject);
}

pub trait StreamErrorState: Send {
    fn observe_chunk(&mut self, item: &StreamItem);
    fn mark_emitted(&mut self, frame: String) -> String;
    fn format_failure(&mut self, failure: &StreamFailure) -> Option<String>;
}

pub trait ErrorStateSource: Send + Sync {
    fn new_state(&self) -> Box<dyn StreamErrorState + Send>;
}

pub struct CleanupOutcome<Key> {
    pub request: Option<Parts>,
    pub request_data: JsonObject,
    pub key: Key,
    pub response: Option<Arc<dyn StreamSnapshot>>,
    pub stream_completed: bool,
    pub client_disconnected: bool,
}

pub trait StreamCleanup<Key>: Send + Sync {
    fn finalize(&self, outcome: CleanupOutcome<Key>);
}

pub struct SseDataGenerator<Key> {
    hooks: Arc<dyn StreamHooks<Key>>,
    error_state: Arc<dyn ErrorStateSource>,
    cleanup: Arc<dyn StreamCleanup<Key>>,
    router: Option<Arc<dyn KeepaliveRouter>>,
    global_keepalive: Option<PingSetting>,
    expose_fallback_errors: bool,
    is_model_router: RouterCheck,
}

impl<Key> SseDataGenerator<Key> {
    pub fn new(
        hooks: Arc<dyn StreamHooks<Key>>,
        error_state: Arc<dyn ErrorStateSource>,
        cleanup: Arc<dyn StreamCleanup<Key>>,
        router: Option<Arc<dyn KeepaliveRouter>>,
        global_keepalive: Option<PingSetting>,
        expose_fallback_errors: bool,
        is_model_router: RouterCheck,
    ) -> Self {
        Self {
            hooks,
            error_state,
            cleanup,
            router,
            global_keepalive,
            expose_fallback_errors,
            is_model_router,
        }
    }
}

impl<Key: Send + 'static> AsyncDataGenerator<Key> for SseDataGenerator<Key> {
    type Response = StreamSource;
    type Stream = BoxStream<'static, Result<Bytes, io::Error>>;

    fn generate(&self, input: GeneratorInput<Self::Response, Key>) -> Self::Stream {
        let GeneratorInput {
            response,
            key,
            request_data,
            request,
            responses_stream_errors,
        } = input;
        let requested = client_requested_model(&request_data);
        let mut fallback = fallback_metadata(&response.hidden);
        let include = include_fallback_errors(self.expose_fallback_errors, &request_data);
        let raw = flag(&request_data, "_litellm_raw_sse_stream");
        let strip = flag(&request_data, "_litellm_strip_stream_usage");
        let skip_done = flag(&request_data, "_litellm_skip_openai_stream_done");
        let wrapped = self.hooks.needs_iterator_wrap();
        let mut items = response.items;
        if wrapped {
            items = self.hooks.wrap_iterator(items, &key, &request_data);
        }
        let mut resolver = KeepaliveResolver::new(
            self.router.clone(),
            Arc::new(request_data.clone()),
            self.global_keepalive.clone(),
        );
        let initial = resolver.resolve(&response.hidden);
        let use_keepalive = self.router.is_some() || initial > 0.0_f64;
        let beats: BoxStream<'static, Beat<Result<StreamItem, StreamFailure>>> = if use_keepalive {
            Box::pin(with_keepalive(items, resolver, initial))
        } else {
            Box::pin(items.map(Beat::Item))
        };
        let error_state = if responses_stream_errors {
            Some(self.error_state.new_state())
        } else {
            None
        };
        let mut guard = CleanupGuard {
            cleanup: self.cleanup.clone(),
            outcome: Some(CleanupOutcome {
                request,
                request_data: request_data.clone(),
                key,
                response: response.snapshot,
                stream_completed: false,
                client_disconnected: false,
            }),
        };
        let hooks = self.hooks.clone();
        let is_model_router = self.is_model_router;
        let mut chunk_text = String::new();
        let mut raw_buffer = String::new();
        let mut fallback_sent = false;
        let mut mismatch = false;
        let mut beats = beats;
        let mut error_message = None;
        let mut done = false;
        let body = stream::unfold((), move |_| {
            let mut result = None;
            async {
                if done {
                    return None;
                }
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
                            result = Some(Ok(Bytes::from(frame)));
                        } else if let Some(message) = error_message.take() {
                            result = Some(Ok(Bytes::from(sse_data_frame(&message))));
                        } else if !skip_done {
                            result = Some(Ok(Bytes::from(DONE_FRAME)));
                        } else {
                            done = true;
                        }
                        return result.map(|item| (item, ()));
                    };
                    match beat {
                        Beat::Ping => return Some((Ok(Bytes::from_static(b": ping\n\n")), ())),
                        Beat::Item(Err(failure)) => {
                            tracing::error!(
                                "litellm.proxy.proxy_server.async_data_generator(): Exception occured - {failure}"
                            );
                            let hooked = hooks
                                .failure_hook(
                                    &guard.outcome.as_ref().expect("active cleanup").key,
                                    &failure,
                                    &request_data,
                                )
                                .await;
                            if let Err(hook_failure) = hooked {
                                guard.complete(true);
                                done = true;
                                return Some((Err(io::Error::other(hook_failure.message)), ()));
                            }
                            let frame = if let Some(state) = error_state.as_mut() {
                                guard.complete(true);
                                state.format_failure(&failure).map(|frame| {
                                    let mut output = frame;
                                    if !skip_done {
                                        output.push_str(DONE_FRAME);
                                    }
                                    output
                                })
                            } else if failure.kind == FailureKind::Http {
                                guard.complete(true);
                                done = true;
                                return Some((Err(io::Error::other(failure.message)), ()));
                            } else {
                                guard.complete(true);
                                Some(failure_frame(&failure))
                            };
                            done = true;
                            return frame.map(|frame| (Ok(Bytes::from(frame)), ()));
                        }
                        Beat::Item(Ok(mut item)) => {
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
                                        done = true;
                                        return Some((Err(io::Error::other(failure.message)), ()));
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
                                    return Some((
                                        Ok(Bytes::from(fallback_event_frame(
                                            fallback.model.as_deref(),
                                            &fallback.errors,
                                            0,
                                        ))),
                                        (),
                                    ));
                                }
                                continue;
                            }
                            if let Some(state) = error_state.as_mut() {
                                state.observe_chunk(&item);
                            }
                            let frame = match item {
                                StreamItem::Chunk(chunk) => {
                                    Some(sse_data_frame(&serialize_chunk(&chunk)))
                                }
                                StreamItem::Bytes(bytes) => {
                                    let text = String::from_utf8_lossy(&bytes).into_owned();
                                    if raw {
                                        raw_buffer.push_str(&text);
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
                                    None
                                }
                                StreamItem::Text(text) if text.starts_with("data: ") => {
                                    error_message = Some(text);
                                    done = true;
                                    None
                                }
                                StreamItem::Text(text) => Some(sse_data_frame(&text)),
                            };
                            if raw {
                                let (complete, rest) = pop_sse_frame(&raw_buffer);
                                raw_buffer = rest;
                                if let Some(frame) = complete {
                                    return Some((Ok(Bytes::from(frame)), ()));
                                }
                                if raw_buffer.chars().count() > MAX_RAW_SSE_BUFFER_CHARS {
                                    guard.complete(true);
                                    done = true;
                                    return Some((
                                        Err(io::Error::other(
                                            "Raw SSE stream exceeded maximum buffered size without a frame delimiter",
                                        )),
                                        (),
                                    ));
                                }
                            }
                            if let Some(mut frame) = frame {
                                if let Some(state) = error_state.as_mut() {
                                    frame = state.mark_emitted(frame);
                                }
                                if pending {
                                    fallback_sent = true;
                                    frame.push_str(&fallback_event_frame(
                                        fallback.model.as_deref(),
                                        &fallback.errors,
                                        0,
                                    ));
                                }
                                return Some((Ok(Bytes::from(frame)), ()));
                            }
                        }
                    }
                }
            }
        });
        body.boxed()
    }
}

impl KeepaliveSource for Result<StreamItem, StreamFailure> {
    fn hidden_params(&self) -> Option<&JsonObject> {
        self.as_ref().ok().and_then(StreamItem::hidden_params)
    }
}

struct CleanupGuard<Key> {
    cleanup: Arc<dyn StreamCleanup<Key>>,
    outcome: Option<CleanupOutcome<Key>>,
}

impl<Key> CleanupGuard<Key> {
    fn complete(&mut self, stream_completed: bool) {
        if let Some(mut outcome) = self.outcome.take() {
            outcome.stream_completed = stream_completed;
            outcome.client_disconnected = false;
            self.cleanup.finalize(outcome);
        }
    }
}

impl<Key> Drop for CleanupGuard<Key> {
    fn drop(&mut self) {
        if let Some(mut outcome) = self.outcome.take() {
            outcome.client_disconnected = !outcome.stream_completed;
            self.cleanup.finalize(outcome);
        }
    }
}

fn flag(data: &JsonObject, key: &str) -> bool {
    matches!(data.get(key), Some(Json::Bool(true)))
}

fn failure_frame(failure: &StreamFailure) -> String {
    let code = if failure.message.contains("No healthy deployment available")
        || failure.message.contains("No deployments available")
    {
        429
    } else if failure
        .message
        .contains("Not allowed to access model due to tags configuration")
    {
        401
    } else {
        failure.status_code.unwrap_or(500)
    };
    let mut error = JsonObject::new();
    error.insert("message".to_owned(), Json::Text(failure.message.clone()));
    error.insert(
        "type".to_owned(),
        Json::Text(
            failure
                .error_type
                .clone()
                .unwrap_or_else(|| "None".to_owned()),
        ),
    );
    error.insert(
        "param".to_owned(),
        match &failure.param {
            FailureParam::Missing => Json::Text("None".to_owned()),
            FailureParam::Null => Json::Null,
            FailureParam::Text(value) => Json::Text(value.clone()),
        },
    );
    error.insert("code".to_owned(), Json::Text(code.to_string()));
    let mut root = JsonObject::new();
    root.insert("error".to_owned(), Json::Object(error));
    format!(
        "data: {}\n\n",
        serde_json::to_string(&Json::Object(root)).unwrap_or_default()
    )
}

#[cfg(test)]
mod tests;
