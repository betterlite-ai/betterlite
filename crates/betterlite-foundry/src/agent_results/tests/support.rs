#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_prompts::arguments::json::Object;
use betterlite_tokenizer::count::error::CountError;
use futures_util::stream::{BoxStream, iter};
use futures_util::{Stream, StreamExt as _};
use secrecy::ExposeSecret as _;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};
use uuid::Uuid;

use crate::agent_requests::prepare::Headers;
use crate::agent_results::error::AgentResultError;
use crate::agent_results::messages::{ASSISTANT_ROLE, MessagesData};
use crate::agent_results::reply::{STOP_FINISH_REASON, TokenCountRequest, TokenCounter};
use crate::agent_results::stream::{
    AgentChunk, AgentChunkStream, CHUNK_OBJECT, ChunkChoice, ChunkDelta, STATUS_OK, StreamIdentity,
    StreamSetup, open_stream, response_identity,
};

pub(super) type Body = BoxStream<'static, Result<Vec<u8>, HttpError>>;

pub(super) const CREATED: i64 = 1_700_000_000;
pub(super) const MODEL: &str = "azure_ai/agents/a";

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

// SecretString has no PartialEq, so a test compares the exposed names and values instead
pub(super) fn shown_headers(headers: &Headers) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

pub(super) fn messages(text: &str) -> MessagesData {
    serde_json::from_str(text).expect("messages data")
}

pub(super) fn identity() -> StreamIdentity {
    response_identity(
        Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0001),
        CREATED,
    )
}

pub(super) fn setup(status: u16) -> StreamSetup<'static> {
    StreamSetup {
        model: MODEL,
        status,
        identity: identity(),
    }
}

pub(super) async fn stream_for(body: &str) -> AgentChunkStream {
    open_stream(setup(STATUS_OK), whole(body))
        .await
        .expect("a stream")
}

pub(super) async fn stream_for_chunks(chunks: Vec<Vec<u8>>) -> AgentChunkStream {
    open_stream(setup(STATUS_OK), body_stream(chunks))
        .await
        .expect("a stream")
}

pub(super) async fn results_of(
    stream: AgentChunkStream,
) -> Vec<Result<AgentChunk, AgentResultError>> {
    stream.collect().await
}

pub(super) async fn chunks_of(stream: AgentChunkStream) -> Vec<AgentChunk> {
    results_of(stream)
        .await
        .into_iter()
        .map(|chunk| chunk.expect("a chunk"))
        .collect()
}

pub(super) fn delta_chunk(text: &str) -> AgentChunk {
    AgentChunk {
        id: identity().id,
        created: CREATED,
        model: String::from(MODEL),
        object: CHUNK_OBJECT,
        choices: vec![ChunkChoice {
            index: 0,
            finish_reason: None,
            delta: ChunkDelta {
                content: Some(text.to_owned()),
                role: Some(ASSISTANT_ROLE),
                annotations: None,
            },
        }],
        thread_id: None,
    }
}

pub(super) fn terminal_chunk(
    annotations: Option<Vec<Object>>,
    thread_id: Option<String>,
) -> AgentChunk {
    AgentChunk {
        id: identity().id,
        created: CREATED,
        model: String::from(MODEL),
        object: CHUNK_OBJECT,
        choices: vec![ChunkChoice {
            index: 0,
            finish_reason: Some(STOP_FINISH_REASON),
            delta: ChunkDelta {
                content: None,
                role: None,
                annotations,
            },
        }],
        thread_id,
    }
}

pub(super) fn block_on<Output>(future: impl Future<Output = Output>) -> Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(future)
}

pub(super) fn body_stream(chunks: Vec<Vec<u8>>) -> Body {
    iter(chunks.into_iter().map(Ok::<Vec<u8>, HttpError>)).boxed()
}

pub(super) fn split_bytes(body: &[u8], split_at: &[usize]) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    let mut start = 0_usize;
    for end in split_at {
        let end = (*end).min(body.len());
        if end > start {
            chunks.push(body.get(start..end).unwrap_or_default().to_vec());
            start = end;
        }
    }
    if start < body.len() {
        chunks.push(body.get(start..).unwrap_or_default().to_vec());
    }
    chunks
}

pub(super) fn whole(body: &str) -> Body {
    body_stream(vec![body.as_bytes().to_vec()])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Recorded {
    pub(super) model: String,
    pub(super) messages: Option<Vec<Object>>,
    pub(super) text: Option<String>,
    pub(super) count_response_tokens: bool,
}

pub(super) struct FakeCounter {
    pub(super) prompt: u64,
    pub(super) completion: u64,
    pub(super) fail: bool,
    pub(super) fail_completion: bool,
    pub(super) calls: Mutex<Vec<Recorded>>,
}

impl FakeCounter {
    pub(super) fn counting(prompt: u64, completion: u64) -> Self {
        Self {
            prompt,
            completion,
            fail: false,
            fail_completion: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            prompt: 0,
            completion: 0,
            fail: true,
            fail_completion: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing_completion() -> Self {
        Self {
            prompt: 7,
            completion: 0,
            fail: false,
            fail_completion: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn records(&self) -> Vec<Recorded> {
        self.calls.lock().expect("the call log").clone()
    }
}

#[async_trait]
impl TokenCounter for FakeCounter {
    async fn count_request(&self, request: &TokenCountRequest<'_>) -> Result<u64, CountError> {
        self.calls.lock().expect("the call log").push(Recorded {
            model: request.model.to_owned(),
            messages: request.messages.map(<[Object]>::to_vec),
            text: request.text.map(str::to_owned),
            count_response_tokens: request.count_response_tokens,
        });
        if self.fail || (self.fail_completion && request.text.is_some()) {
            return Err(CountError::MissingInput);
        }
        if request.messages.is_some() {
            Ok(self.prompt)
        } else {
            Ok(self.completion)
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<String>>>);

impl Captured {
    pub(super) fn messages(&self) -> Vec<String> {
        self.0.lock().expect("the captured warnings").clone()
    }
}

#[derive(Default)]
struct Text(Option<String>);

impl Visit for Text {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut text = Text::default();
        event.record(&mut text);
        if let Some(message) = text.0 {
            self.0.lock().expect("the captured warnings").push(message);
        }
    }
}

pub(super) async fn with_warnings<Output>(
    future: impl Future<Output = Output>,
) -> (Output, Vec<String>) {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    let guard = tracing::subscriber::set_default(subscriber);
    let output = future.await;
    drop(guard);
    (output, captured.messages())
}

// the body hands out its items one poll at a time and says when the stream was dropped
pub(super) struct Watched {
    pub(super) remaining: VecDeque<Result<Vec<u8>, HttpError>>,
    pub(super) polls: Arc<AtomicUsize>,
    pub(super) dropped: Arc<AtomicBool>,
}

impl Stream for Watched {
    type Item = Result<Vec<u8>, HttpError>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(self.remaining.pop_front())
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

pub(super) fn watched(
    chunks: Vec<Vec<u8>>,
    polls: &Arc<AtomicUsize>,
    dropped: &Arc<AtomicBool>,
) -> Body {
    Watched {
        remaining: chunks.into_iter().map(Ok).collect(),
        polls: Arc::clone(polls),
        dropped: Arc::clone(dropped),
    }
    .boxed()
}
