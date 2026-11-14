use std::collections::BTreeMap;

use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::voyage_embedding::{
    VoyageEmbeddingError, VoyageEmbeddingVariant, decode_voyage_embedding_reply, endpoint,
};
use betterlite_openai::voyage_multimodal::{self, VoyageMultimodalError};
use betterlite_openai::voyage_rerank::{
    self, RerankLogging, VoyageRerankControls, VoyageRerankError,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::helpers::OrHelperError as _;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    multimodal: Vec<MultimodalCall>,
    rerank: Vec<RerankCall>,
}

#[derive(Deserialize)]
struct MultimodalCall {
    model: String,
    input: Value,
    #[serde(default)]
    options: Map<String, Value>,
    base: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    reply: Reply,
}

#[derive(Deserialize)]
struct RerankCall {
    model: String,
    query: String,
    documents: Vec<RerankDocument>,
    top_n: Option<i64>,
    return_documents: Option<bool>,
    rank_fields: Option<Vec<String>>,
    max_chunks_per_doc: Option<i64>,
    max_tokens_per_doc: Option<i64>,
    instruction: Option<String>,
    base: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    headers: Map<String, Value>,
    reply: Reply,
    uuid: String,
}

#[derive(Deserialize)]
struct Reply {
    body: String,
    status: u16,
}

struct Failure {
    status: Option<u16>,
}

impl From<VoyageMultimodalError> for Failure {
    fn from(_error: VoyageMultimodalError) -> Self {
        Self { status: None }
    }
}

impl From<VoyageEmbeddingError> for Failure {
    fn from(error: VoyageEmbeddingError) -> Self {
        match error {
            VoyageEmbeddingError::Decode { status, .. } => Self {
                status: Some(status),
            },
        }
    }
}

impl From<VoyageRerankError> for Failure {
    fn from(error: VoyageRerankError) -> Self {
        match error {
            VoyageRerankError::MissingApiKey | VoyageRerankError::NoResults => {
                Self { status: None }
            }
            VoyageRerankError::Status { status } | VoyageRerankError::Decode { status, .. } => {
                Self {
                    status: Some(status),
                }
            }
            VoyageRerankError::ResponseField { .. } => Self { status: Some(502) },
        }
    }
}

struct Recorder {
    calls: usize,
}

impl RerankLogging for Recorder {
    fn post_call(&mut self, _original_response: &[u8]) {
        self.calls += 1;
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let multimodal = input
            .multimodal
            .iter()
            .map(multimodal_call)
            .collect::<Result<Vec<_>, Box<crate::helpers::HelperError>>>()?;
        let rerank = input
            .rerank
            .iter()
            .map(rerank_call)
            .collect::<Result<Vec<_>, Box<crate::helpers::HelperError>>>()?;
        Ok(json!({"multimodal": multimodal, "rerank": rerank}))
    }
}

// a reply transform that raises is the whole call's failure, as dispatch.py reports it
fn top_error<E>(error: E) -> Box<crate::helpers::HelperError>
where
    E: Into<Failure>,
{
    let failure: Failure = error.into();
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: failure.status,
        error_type: None,
        param: None,
        code: None,
        message: String::new(),
    })
}

fn outcome<T, E>(result: Result<T, E>) -> crate::helpers::HelperResult
where
    T: serde::Serialize,
    E: Into<Failure>,
{
    match result {
        Ok(value) => Ok(json!({"ok": true, "value": serde_json::to_value(value)?})),
        Err(error) => Ok(json!({"ok": false, "status": error.into().status})),
    }
}

fn multimodal_call(call: &MultimodalCall) -> crate::helpers::HelperResult {
    let request = voyage_multimodal::request(&call.model, call.input.clone(), &call.options);
    let response = decode_voyage_embedding_reply(
        call.reply.body.as_bytes(),
        call.reply.status,
        VoyageEmbeddingVariant::Multimodal,
    )
    .map_err(top_error)?;
    Ok(json!({
        "is_multimodal": voyage_multimodal::is_multimodal_embeddings(&call.model),
        "supported_params": voyage_multimodal::SUPPORTED_OPENAI_PARAMS,
        "url": endpoint(call.base.as_deref(), VoyageEmbeddingVariant::Multimodal),
        "request": outcome(request)?,
        "response": {"ok": true, "value": response},
    }))
}

fn rerank_call(call: &RerankCall) -> crate::helpers::HelperResult {
    let uuid = Uuid::parse_str(&call.uuid).or_value_error()?;
    let headers =
        voyage_rerank::api_key(call.api_key.as_deref(), |name| call.env.get(name).cloned())
            .map(|key| voyage_rerank::headers(call.headers.clone(), &key));
    let controls = VoyageRerankControls {
        top_n: call.top_n,
        return_documents: call.return_documents,
        rank_fields: call.rank_fields.clone(),
        max_chunks_per_doc: call.max_chunks_per_doc,
        max_tokens_per_doc: call.max_tokens_per_doc,
        instruction: call.instruction.clone(),
    };
    let request = voyage_rerank::request(&call.model, &call.query, &call.documents, &controls);
    let mut recorder = Recorder { calls: 0 };
    let response = voyage_rerank::decode_voyage_rerank(
        call.reply.body.as_bytes(),
        call.reply.status,
        uuid,
        &mut recorder,
    )
    .map_err(top_error)?;
    Ok(json!({
        "supported_params": voyage_rerank::SUPPORTED_COHERE_RERANK_PARAMS,
        "headers": outcome(headers)?,
        "url": voyage_rerank::endpoint(call.base.as_deref()),
        "request": outcome(Ok::<_, Failure>(request))?,
        "response": {"ok": true, "value": serde_json::to_value(response)?},
        "post_call": recorder.calls > 0,
    }))
}

#[path = "voyage_media_rank/tests.rs"]
mod tests;
