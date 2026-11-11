use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_bedrock::agentcore_stream::adapter::open;
use betterlite_bedrock::agentcore_stream::transport::{
    Chunks, HandlerKind, PostRequest, StreamHandler, StreamHead,
};
use betterlite_bedrock::agentcore_stream::{
    AgentCoreStream, AgentCoreStreamError, IdSource, StreamDeps, StreamLogger, StreamPostCall,
    StreamRequest,
};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::stream::chunk::Chunk;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::{HeaderMap, HeaderName, HeaderValue};
use uuid::Uuid;

use super::Scenario;
use super::failure;
use crate::helpers::HelperError;

pub(super) struct FixedIds {
    values: Mutex<VecDeque<Uuid>>,
}

impl FixedIds {
    fn new(values: &[Uuid]) -> Self {
        Self {
            values: Mutex::new(values.iter().copied().collect()),
        }
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or_else(Uuid::nil)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Recorded {
    pub(super) url: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) stream: bool,
}

struct FixtureHandler {
    kind: HandlerKind,
    status: u16,
    headers: HeaderMap,
    reads: Vec<String>,
    posts: Mutex<Vec<Recorded>>,
}

impl FixtureHandler {
    fn new(kind: HandlerKind, scenario: &Scenario) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            kind,
            status: scenario.status,
            headers: header_map(&scenario.headers)?,
            reads: scenario.reads.clone(),
            posts: Mutex::new(Vec::new()),
        })
    }

    fn posts(&self) -> Vec<Recorded> {
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
            .push(Recorded {
                url: request.url.to_string(),
                headers: header_pairs(&request.headers),
                body: request.body,
                stream: request.stream,
            });
        Ok(StreamHead {
            status: self.status,
            headers: self.headers.clone(),
            chunks: chunk_stream(&self.reads),
        })
    }
}

fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.as_str().to_owned(), text.to_owned()))
        })
        .collect()
}

pub(super) fn header_map(pairs: &[(String, String)]) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|error| failure(error.to_string()))?;
        let header_value =
            HeaderValue::from_str(value).map_err(|error| failure(error.to_string()))?;
        headers.append(header_name, header_value);
    }
    Ok(headers)
}

fn chunk_stream(reads: &[String]) -> Chunks {
    let items: Vec<Result<Vec<u8>, HttpError>> = reads
        .iter()
        .map(|read| Ok(read.clone().into_bytes()))
        .collect();
    stream::iter(items).boxed()
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
    pub(super) fn calls(&self) -> Vec<LoggedCall> {
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

pub(super) struct Fixture {
    supplied: Option<Arc<FixtureHandler>>,
    default: Arc<FixtureHandler>,
    pub(super) logger: FixtureLogger,
    ids: Arc<FixedIds>,
}

impl Fixture {
    pub(super) fn new(
        kind: HandlerKind,
        scenario: &Scenario,
        ids: &[Uuid],
    ) -> Result<Self, Box<HelperError>> {
        let supplied = match scenario.supplied {
            Some(supplied) => Some(Arc::new(FixtureHandler::new(
                super::handler_kind(supplied),
                scenario,
            )?)),
            None => None,
        };
        Ok(Self {
            supplied,
            default: Arc::new(FixtureHandler::new(kind, scenario)?),
            logger: FixtureLogger::default(),
            ids: Arc::new(FixedIds::new(ids)),
        })
    }

    pub(super) async fn open(
        &self,
        request: &StreamRequest<'_>,
        kind: HandlerKind,
    ) -> Result<AgentCoreStream, AgentCoreStreamError> {
        let supplied: Option<Arc<dyn StreamHandler>> = self.supplied.as_ref().map(|handler| {
            let cloned: Arc<dyn StreamHandler> = Arc::<FixtureHandler>::clone(handler);
            cloned
        });
        let default: Arc<dyn StreamHandler> = Arc::<FixtureHandler>::clone(&self.default);
        let ids: Arc<dyn IdSource> = Arc::<FixedIds>::clone(&self.ids);
        let deps = StreamDeps {
            supplied,
            default,
            ids,
            logger: &self.logger,
        };
        open(request, &deps, kind).await
    }

    pub(super) fn posts(&self) -> Vec<Recorded> {
        let mut posts = self.default.posts();
        if let Some(supplied) = &self.supplied {
            posts.extend(supplied.posts());
        }
        posts
    }
}

pub(super) async fn drain(
    stream: &mut AgentCoreStream,
) -> Result<Vec<Chunk>, AgentCoreStreamError> {
    let mut chunks = Vec::new();
    loop {
        match stream.next_chunk().await {
            Ok(Some(chunk)) => chunks.push(chunk),
            Ok(None) => break Ok(chunks),
            Err(error) => break Err(error),
        }
    }
}
