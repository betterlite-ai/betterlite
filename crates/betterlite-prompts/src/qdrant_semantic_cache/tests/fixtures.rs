use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_http::proxy::transport_error::TransportError;
use tracing::subscriber::{DefaultGuard, set_default};
use uuid::Uuid;

use crate::arguments::json::{Json, Object};
use crate::message_content::{Content, Message};
use crate::qdrant_semantic_cache::error::QdrantSemanticError;
use crate::qdrant_semantic_cache::options::{QdrantEnv, QdrantSemanticOptions};
use crate::qdrant_semantic_cache::ports::{
    QdrantEmbedError, QdrantEmbedder, QdrantReply, QdrantRequest, QdrantTransport, UuidSource,
};
use crate::qdrant_semantic_cache::{QdrantCacheKwargs, QdrantSemanticCache};

pub(crate) const VECTOR: [f64; 3] = [0.25, 0.5, 0.75];
pub(crate) const FIXED_ID: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
pub(crate) const SIMILARITY: &str = "semantic-similarity";

pub(crate) enum Reply {
    Answer(QdrantReply),
    Fail,
}

#[derive(Default)]
pub(crate) struct RequestLog(Mutex<Vec<QdrantRequest>>);

impl RequestLog {
    fn record(&self, request: QdrantRequest) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }

    pub(crate) fn requests(&self) -> Vec<QdrantRequest> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// Scripted replies answer first; once they run out the transport keeps the points and searches them.
pub(crate) struct FakeQdrant {
    pub(crate) score: f64,
    replies: Mutex<VecDeque<Reply>>,
    payloads: Mutex<Vec<Object>>,
    log: RequestLog,
}

impl FakeQdrant {
    pub(crate) fn new(score: f64, replies: Vec<Reply>) -> Self {
        Self {
            score,
            replies: Mutex::new(VecDeque::from(replies)),
            payloads: Mutex::new(Vec::new()),
            log: RequestLog::default(),
        }
    }

    pub(crate) fn requests(&self) -> Vec<QdrantRequest> {
        self.log.requests()
    }

    pub(crate) fn payloads(&self) -> Vec<Object> {
        self.payloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn store(&self, body: Option<&Json>) -> QdrantReply {
        let Some(Json::List(points)) = object_field(body, "points") else {
            return failed_reply();
        };
        let mut stored = Vec::new();
        for point in points {
            let Some(payload) = payload_of(point) else {
                return failed_reply();
            };
            stored.push(payload);
        }
        self.payloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(stored);
        QdrantReply {
            status: 200,
            body: br#"{"result":true}"#.to_vec(),
        }
    }

    fn search(&self, body: Option<&Json>) -> QdrantReply {
        let Some(key) = filter_key(body) else {
            return failed_reply();
        };
        let found = {
            let payloads = self.payloads.lock().unwrap_or_else(PoisonError::into_inner);
            payloads
                .iter()
                .find(|payload| {
                    payload.get("litellm_cache_key").and_then(Json::as_str) == Some(key)
                })
                .cloned()
        };
        let Some(payload) = found else {
            return QdrantReply {
                status: 200,
                body: br#"{"result":[]}"#.to_vec(),
            };
        };
        let hit = Json::Object(Object::from([
            (
                "score".to_owned(),
                Json::Number(serde_json::Number::from_f64(self.score).expect("a finite score")),
            ),
            ("payload".to_owned(), Json::Object(payload)),
        ]));
        let reply = Json::Object(Object::from([("result".to_owned(), Json::List(vec![hit]))]));
        QdrantReply {
            status: 200,
            body: serde_json::to_vec(&reply).expect("the search reply"),
        }
    }
}

#[async_trait]
impl QdrantTransport for FakeQdrant {
    async fn send(&self, request: QdrantRequest) -> Result<QdrantReply, QdrantSemanticError> {
        let path = request.path.clone();
        let body = request
            .body
            .as_ref()
            .map(|bytes| serde_json::from_slice::<Json>(bytes).expect("a JSON request body"));
        let scripted = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        self.log.record(request);
        match scripted {
            Some(Reply::Answer(answer)) => Ok(answer),
            Some(Reply::Fail) => Err(QdrantSemanticError::Transport {
                path,
                source: TransportError::Closed,
            }),
            None if path.ends_with("/points/search") => Ok(self.search(body.as_ref())),
            None => Ok(self.store(body.as_ref())),
        }
    }
}

fn object_field<'input>(body: Option<&'input Json>, name: &str) -> Option<&'input Json> {
    let fields = body.and_then(Json::as_object)?;
    fields.get(name)
}

fn payload_of(point: &Json) -> Option<Object> {
    let fields = point.as_object()?;
    fields.get("payload").and_then(Json::as_object).cloned()
}

fn filter_key(body: Option<&Json>) -> Option<&str> {
    let Json::List(conditions) = object_field(object_field(body, "filter"), "must")? else {
        return None;
    };
    let condition = conditions.first()?;
    let value = object_field(Some(condition), "match")?;
    object_field(Some(value), "value").and_then(Json::as_str)
}

fn failed_reply() -> QdrantReply {
    QdrantReply {
        status: 500,
        body: br#"{"status":"error"}"#.to_vec(),
    }
}

pub(crate) struct ConstantEmbedder {
    pub(crate) vector: Vec<f64>,
    pub(crate) failure: Option<String>,
    pub(crate) calls: Mutex<Vec<(String, Option<Object>)>>,
}

impl ConstantEmbedder {
    pub(crate) fn new() -> Self {
        Self::with_vector(VECTOR.to_vec())
    }

    pub(crate) fn with_vector(vector: Vec<f64>) -> Self {
        Self {
            vector,
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            vector: VECTOR.to_vec(),
            failure: Some("the embedding provider failed".to_owned()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<(String, Option<Object>)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl QdrantEmbedder for ConstantEmbedder {
    async fn embed(
        &self,
        prompt: &str,
        metadata: Option<&Object>,
    ) -> Result<Vec<f64>, QdrantEmbedError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((prompt.to_owned(), metadata.cloned()));
        if let Some(message) = &self.failure {
            return Err(QdrantEmbedError::Failed {
                message: message.clone(),
            });
        }
        Ok(self.vector.clone())
    }
}

pub(crate) struct FixedUuids(pub(crate) Uuid);

impl UuidSource for FixedUuids {
    fn next_id(&self) -> Uuid {
        self.0
    }
}

pub(crate) fn fixed_uuids() -> FixedUuids {
    FixedUuids(Uuid::from_u128(FIXED_ID))
}

pub(crate) fn options() -> QdrantSemanticOptions {
    QdrantSemanticOptions {
        collection_name: Some("semantic-cache".to_owned()),
        similarity_threshold: Some(0.8_f64),
        qdrant_api_base: Some("http://qdrant.test".to_owned()),
        qdrant_api_key: None,
        quantization_config: None,
        embedding_model: None,
        vector_size: None,
        scalar_quantile: None,
        embedding_max_input_tokens: None,
        embedding_timeout: None,
    }
}

pub(crate) fn default_env() -> QdrantEnv {
    QdrantEnv::default()
}

pub(crate) fn env_with(values: &[(&str, &str)]) -> QdrantEnv {
    QdrantEnv {
        values: values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<BTreeMap<_, _>>(),
        vector_size: None,
        scalar_quantile: None,
    }
}

pub(crate) fn build(
    options: QdrantSemanticOptions,
    env: &QdrantEnv,
    transport: Arc<FakeQdrant>,
    embedder: Arc<ConstantEmbedder>,
) -> Result<QdrantSemanticCache, QdrantSemanticError> {
    let backend: Arc<dyn QdrantTransport> = transport;
    QdrantSemanticCache::new(options, env, embedder, backend, Arc::new(fixed_uuids()))
}

pub(crate) fn cache_from(
    transport: Arc<dyn QdrantTransport>,
    embedder: Arc<ConstantEmbedder>,
) -> QdrantSemanticCache {
    QdrantSemanticCache::new(
        options(),
        &default_env(),
        embedder,
        transport,
        Arc::new(fixed_uuids()),
    )
    .expect("a cache built from the fixture options")
}

pub(crate) fn cache_with(
    transport: Arc<FakeQdrant>,
    embedder: Arc<ConstantEmbedder>,
) -> QdrantSemanticCache {
    let backend: Arc<dyn QdrantTransport> = transport;
    cache_from(backend, embedder)
}

pub(crate) fn kwargs() -> QdrantCacheKwargs {
    QdrantCacheKwargs {
        messages: Some(vec![user_message("what is cached")]),
        metadata: None,
    }
}

pub(crate) fn user_message(content: &str) -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Text(content.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

pub(crate) fn answer(status: u16, body: &str) -> Reply {
    Reply::Answer(QdrantReply {
        status,
        body: body.as_bytes().to_vec(),
    })
}

pub(crate) fn exists(exists: bool) -> Reply {
    let value = if exists { "true" } else { "false" };
    answer(200, &format!(r#"{{"result":{{"exists":{value}}}}}"#))
}

pub(crate) fn created(result: bool) -> Reply {
    answer(200, &format!(r#"{{"result":{result}}}"#))
}

pub(crate) fn details() -> Reply {
    answer(200, r#"{"result":{"status":"green"}}"#)
}

pub(crate) fn index_created() -> Reply {
    answer(200, r#"{"result":true}"#)
}

pub(crate) fn search_hit(score: f64, key: Option<&str>, response: &str) -> Reply {
    let key_field = key.map_or_else(String::new, |value| {
        format!(r#""litellm_cache_key":"{value}","#)
    });
    let body = format!(
        r#"{{"result":[{{"score":{score},"payload":{{{key_field}"text":"what is cached","response":{response}}}}}]}}"#
    );
    answer(200, &body)
}

pub(crate) fn search_result_null() -> Reply {
    answer(200, r#"{"result":null}"#)
}

pub(crate) fn body_of(request: &QdrantRequest) -> Json {
    let body = request.body.as_ref().expect("a request body");
    serde_json::from_slice(body).expect("a JSON request body")
}

pub(crate) fn field<'input>(value: &'input Json, name: &str) -> &'input Json {
    value
        .as_object()
        .expect("a JSON object")
        .get(name)
        .expect("the named field")
}

pub(crate) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("fixture JSON")
}

pub(crate) fn metadata() -> Object {
    Object::from([("request-id".to_owned(), Json::Text("abc".to_owned()))])
}

pub(crate) fn similarity_of(metadata: &Object) -> Option<f64> {
    let value = metadata.get(SIMILARITY)?;
    let Json::Number(number) = value else {
        return None;
    };
    number.as_f64()
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("captured UTF-8 logs")
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

pub(crate) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}
