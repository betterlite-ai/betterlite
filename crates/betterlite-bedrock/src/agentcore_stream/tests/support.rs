use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::stream::chunk::Chunk;
use futures_util::StreamExt as _;
use futures_util::stream;
use futures_util::stream::BoxStream;
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use tokio::runtime::Builder;
use tracing::subscriber::{DefaultGuard, set_default};
use uuid::Uuid;

use crate::agentcore_stream::adapter;
use crate::agentcore_stream::transport::{
    Chunks, HandlerKind, PostRequest, StreamHandler, StreamHead,
};
use crate::agentcore_stream::{
    AgentCoreStream, AgentCoreStreamError, IdSource, StreamDeps, StreamLogger, StreamPostCall,
    StreamRequest,
};

pub(super) const MODEL: &str = "agentcore-model";

pub(super) fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

pub(super) fn ids(count: usize) -> Vec<Uuid> {
    (0_u128..u128::try_from(count).unwrap())
        .map(|offset| Uuid::from_u128(10_000_u128.saturating_add(offset)))
        .collect()
}

pub(super) struct FixedIds {
    values: Mutex<Vec<Uuid>>,
}

impl FixedIds {
    pub(super) fn new(values: &[Uuid]) -> Self {
        Self {
            values: Mutex::new(values.to_vec()),
        }
    }

    pub(super) fn remaining(&self) -> usize {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        if values.is_empty() {
            return Uuid::nil();
        }
        values.remove(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordedRequest {
    pub(super) url: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) stream: bool,
    pub(super) deadline: Duration,
}

pub(super) struct FixtureHandler {
    kind: HandlerKind,
    status: u16,
    headers: Vec<(String, String)>,
    reads: Vec<String>,
    posts: Mutex<Vec<RecordedRequest>>,
}

impl FixtureHandler {
    fn new(kind: HandlerKind, scenario: &Scenario) -> Self {
        Self {
            kind,
            status: scenario.status,
            headers: scenario.response_headers.clone(),
            reads: scenario.reads.clone(),
            posts: Mutex::new(Vec::new()),
        }
    }

    fn posts(&self) -> Vec<RecordedRequest> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl StreamHandler for FixtureHandler {
    fn kind(&self) -> HandlerKind {
        self.kind
    }

    async fn post(&self, request: PostRequest) -> Result<StreamHead, TransportError> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedRequest {
                url: request.url.to_string(),
                headers: headers_of(&request.headers),
                body: request.body,
                stream: request.stream,
                deadline: request.deadline,
            });
        Ok(StreamHead {
            status: self.status,
            headers: header_map(&self.headers),
            chunks: chunk_stream(&self.reads),
        })
    }
}

pub(super) fn header_map(pairs: &[(String, String)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    headers
}

pub(super) fn sorted_pairs(pairs: &[(String, String)]) -> Vec<(String, String)> {
    let mut sorted = pairs.to_vec();
    sorted.sort();
    sorted
}

fn headers_of(headers: &HeaderMap) -> Vec<(String, String)> {
    let pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), value.to_str().unwrap().to_owned()))
        .collect();
    sorted_pairs(&pairs)
}

fn chunk_stream(reads: &[String]) -> Chunks {
    let items: Vec<Result<Vec<u8>, HttpError>> = reads
        .iter()
        .map(|read| Ok(read.clone().into_bytes()))
        .collect();
    let chunks: BoxStream<'static, Result<Vec<u8>, HttpError>> = stream::iter(items).boxed();
    chunks
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoggedCall {
    pub(super) input: Json,
    pub(super) api_key: String,
    pub(super) original_response: String,
    pub(super) complete_input_dict: Json,
}

#[derive(Default)]
pub(super) struct FixtureLogger {
    calls: Mutex<Vec<LoggedCall>>,
}

impl FixtureLogger {
    fn calls(&self) -> Vec<LoggedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl StreamLogger for FixtureLogger {
    fn post_call(&self, call: StreamPostCall<'_>) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LoggedCall {
                input: call.input.clone(),
                api_key: call.api_key.to_owned(),
                original_response: call.original_response.to_owned(),
                complete_input_dict: call.complete_input_dict.clone(),
            });
    }
}

#[derive(Debug, Clone)]
pub(super) struct Scenario {
    pub(super) reads: Vec<String>,
    pub(super) status: u16,
    pub(super) response_headers: Vec<(String, String)>,
    pub(super) api_base: String,
    pub(super) request_headers: Vec<(String, String)>,
    pub(super) data: Json,
    pub(super) signed_json_body: Option<Vec<u8>>,
    pub(super) messages: Json,
    pub(super) supplied: Option<HandlerKind>,
}

impl Scenario {
    fn with(reads: Vec<String>, response_headers: Vec<(String, String)>) -> Self {
        Self {
            reads,
            status: 200,
            response_headers,
            api_base: "https://bedrock-agentcore.example/runtime".to_owned(),
            request_headers: Vec::new(),
            data: Json::Object(IndexMap::default()),
            signed_json_body: None,
            messages: Json::List(Vec::new()),
            supplied: None,
        }
    }

    pub(super) fn sse(reads: &[&str]) -> Self {
        let headers = vec![("content-type".to_owned(), "text/event-stream".to_owned())];
        Self::with(
            reads.iter().map(|read| (*read).to_owned()).collect(),
            headers,
        )
    }

    pub(super) fn json(body: &str) -> Self {
        let headers = vec![("content-type".to_owned(), "application/json".to_owned())];
        Self::with(vec![body.to_owned()], headers)
    }
}

pub(super) struct Outcome {
    pub(super) handler: &'static str,
    pub(super) provider: Option<&'static str>,
    pub(super) posts: Vec<RecordedRequest>,
    pub(super) post_calls: Vec<LoggedCall>,
    pub(super) remaining_ids: usize,
    pub(super) before_error: Vec<Chunk>,
    pub(super) emitted: Result<Vec<Chunk>, AgentCoreStreamError>,
}

struct Fixture {
    supplied: Option<Arc<FixtureHandler>>,
    default: Arc<FixtureHandler>,
    logger: FixtureLogger,
    id_source: Arc<FixedIds>,
    data: Json,
    messages: Json,
}

impl Fixture {
    fn build(kind: HandlerKind, scenario: &Scenario, ids: &[Uuid]) -> Self {
        let supplied = scenario
            .supplied
            .map(|supplied_kind| Arc::new(FixtureHandler::new(supplied_kind, scenario)));
        let default = Arc::new(FixtureHandler::new(kind, scenario));
        let logger = FixtureLogger::default();
        let id_source = Arc::new(FixedIds::new(ids));
        let data = scenario.data.clone();
        let messages = scenario.messages.clone();
        Self {
            supplied,
            default,
            logger,
            id_source,
            data,
            messages,
        }
    }

    fn open(
        &self,
        kind: HandlerKind,
        scenario: &Scenario,
    ) -> Result<AgentCoreStream, AgentCoreStreamError> {
        let supplied: Option<Arc<dyn StreamHandler>> = self.supplied.as_ref().map(|handler| {
            let cloned: Arc<dyn StreamHandler> = Arc::<FixtureHandler>::clone(handler);
            cloned
        });
        let default: Arc<dyn StreamHandler> = Arc::<FixtureHandler>::clone(&self.default);
        let ids: Arc<dyn IdSource> = Arc::<FixedIds>::clone(&self.id_source);
        let request = StreamRequest {
            model: MODEL,
            api_base: &scenario.api_base,
            headers: header_map(&scenario.request_headers),
            data: &self.data,
            signed_json_body: scenario.signed_json_body.as_deref(),
            messages: &self.messages,
        };
        let deps = StreamDeps {
            supplied,
            default,
            ids,
            logger: &self.logger,
        };
        block_on(adapter::open(&request, &deps, kind))
    }
}

pub(super) fn run(kind: HandlerKind, scenario: &Scenario, ids: &[Uuid]) -> Outcome {
    let fixture = Fixture::build(kind, scenario, ids);
    let opened = fixture.open(kind, scenario);
    let (provider, before_error, emitted) = match opened {
        Ok(mut stream) => {
            let provider = stream.provider;
            let (chunks, failure) = drain(&mut stream);
            match failure {
                None => (Some(provider), Vec::new(), Ok(chunks)),
                Some(error) => (Some(provider), chunks, Err(error)),
            }
        }
        Err(error) => (None, Vec::new(), Err(error)),
    };
    let default_posts = fixture.default.posts();
    let handler = if default_posts.is_empty() {
        "supplied"
    } else {
        "default"
    };
    let mut posts = default_posts;
    if let Some(supplied) = &fixture.supplied {
        posts.extend(supplied.posts());
    }
    Outcome {
        handler,
        provider,
        posts,
        post_calls: fixture.logger.calls(),
        remaining_ids: fixture.id_source.remaining(),
        before_error,
        emitted,
    }
}

fn drain(stream: &mut AgentCoreStream) -> (Vec<Chunk>, Option<AgentCoreStreamError>) {
    block_on(async {
        let mut chunks = Vec::new();
        loop {
            match stream.next_chunk().await {
                Ok(Some(chunk)) => chunks.push(chunk),
                Ok(None) => break (chunks, None),
                Err(error) => break (chunks, Some(error)),
            }
        }
    })
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}
