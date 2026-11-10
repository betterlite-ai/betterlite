use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_prompts::qdrant_semantic_cache::error::QdrantSemanticError;
use betterlite_prompts::qdrant_semantic_cache::lookup::SIMILARITY_FIELD;
use betterlite_prompts::qdrant_semantic_cache::options::{QdrantEnv, QdrantSemanticOptions};
use betterlite_prompts::qdrant_semantic_cache::ports::{
    QdrantEmbedError, QdrantEmbedder, QdrantReply, QdrantRequest, QdrantTransport, UuidSource,
};
use betterlite_prompts::qdrant_semantic_cache::{QdrantCacheKwargs, QdrantSemanticCache};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult};

const DEFAULT_API_BASE: &str = "http://qdrant.test";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    key: String,
    collection: String,
    threshold: f64,
    vector: Vec<f64>,
    messages: Vec<Message>,
    #[serde(default)]
    metadata: Option<Object>,
    #[serde(default = "default_api_base")]
    api_base: String,
    responses: Vec<ScriptedResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptedResponse {
    status: u16,
    body: Value,
}

fn default_api_base() -> String {
    DEFAULT_API_BASE.to_owned()
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| helper_error("RuntimeError", error))?;
        runtime.block_on(search(input))
    }
}

struct ConstantEmbedder {
    vector: Vec<f64>,
}

#[async_trait]
impl QdrantEmbedder for ConstantEmbedder {
    async fn embed(
        &self,
        _prompt: &str,
        _metadata: Option<&Object>,
    ) -> Result<Vec<f64>, QdrantEmbedError> {
        Ok(self.vector.clone())
    }
}

struct FixedUuids;

impl UuidSource for FixedUuids {
    fn next_id(&self) -> Uuid {
        Uuid::nil()
    }
}

struct RecordingTransport {
    replies: Mutex<VecDeque<QdrantReply>>,
    requests: Mutex<Vec<QdrantRequest>>,
}

impl RecordingTransport {
    fn new(responses: &[ScriptedResponse]) -> Result<Self, Box<HelperError>> {
        let replies = responses
            .iter()
            .map(|response| {
                let body = serde_json::to_vec(&response.body)
                    .map_err(|error| helper_error("SerializationError", error))?;
                Ok(QdrantReply {
                    status: response.status,
                    body,
                })
            })
            .collect::<Result<VecDeque<_>, Box<HelperError>>>()?;
        Ok(Self {
            replies: Mutex::new(replies),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<QdrantRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl QdrantTransport for RecordingTransport {
    async fn send(&self, request: QdrantRequest) -> Result<QdrantReply, QdrantSemanticError> {
        let path = request.path.clone();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        reply.ok_or(QdrantSemanticError::Transport {
            path,
            source: TransportError::Closed,
        })
    }
}

async fn search(input: &Input) -> HelperResult {
    let transport = Arc::new(RecordingTransport::new(&input.responses)?);
    let cache = build_cache(input, &transport)?;
    cache
        .initialize()
        .await
        .map_err(|error| helper_error("ValueError", error))?;
    let mut metadata = input.metadata.clone();
    let value = cache
        .async_get_cache(&input.key, &kwargs(input), metadata.as_mut())
        .await
        .map_err(|error| helper_error("ValueError", error))?;
    let similarity = metadata
        .as_ref()
        .and_then(|fields| fields.get(SIMILARITY_FIELD))
        .cloned();
    let search = search_record(input, &transport)?;
    Ok(json!({
        "value": value,
        "similarity": similarity,
        "search": search,
    }))
}

fn kwargs(input: &Input) -> QdrantCacheKwargs {
    QdrantCacheKwargs {
        messages: Some(input.messages.clone()),
        metadata: input.metadata.clone(),
    }
}

fn build_cache(
    input: &Input,
    transport: &Arc<RecordingTransport>,
) -> Result<QdrantSemanticCache, Box<HelperError>> {
    let cloned = Arc::clone(transport);
    let backend: Arc<dyn QdrantTransport> = cloned;
    let options = QdrantSemanticOptions {
        collection_name: Some(input.collection.clone()),
        similarity_threshold: Some(input.threshold),
        qdrant_api_base: Some(input.api_base.clone()),
        qdrant_api_key: None,
        quantization_config: None,
        embedding_model: None,
        vector_size: None,
        scalar_quantile: None,
        embedding_max_input_tokens: None,
        embedding_timeout: None,
    };
    QdrantSemanticCache::new(
        options,
        &QdrantEnv::default(),
        Arc::new(ConstantEmbedder {
            vector: input.vector.clone(),
        }),
        backend,
        Arc::new(FixedUuids),
    )
    .map_err(|error| helper_error("ValueError", error))
}

fn search_record(input: &Input, transport: &RecordingTransport) -> HelperResult {
    let found = transport
        .requests()
        .iter()
        .find(|request| request.path.ends_with("/points/search"))
        .cloned();
    let Some(request) = found else {
        return Ok(Value::Null);
    };
    let url = format!("{}{}", input.api_base, request.path);
    let Some(body) = request.body.as_deref() else {
        return Ok(json!({ "url": url, "body": Value::Null }));
    };
    let parsed: Value =
        serde_json::from_slice(body).map_err(|error| helper_error("SerializationError", error))?;
    Ok(json!({ "url": url, "body": parsed }))
}

fn helper_error<M>(class: &str, message: M) -> Box<HelperError>
where
    M: std::fmt::Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
