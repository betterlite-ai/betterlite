use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreCreateResponse;
use serde::{Deserialize, Serialize};

use super::super::error::VertexRagStoreError;
use super::payload::{PayloadError, Reply, parse, provider_error};
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::vertex_fine_tuning::records::timestamp::created_at;

const DEFAULT_CORPUS_NAME: &str = "litellm-vector-store";
const CORPUS_DESCRIPTION: &str = "Vector store created via LiteLLM";
const CORPUS_SUFFIX: &str = "/ragCorpora";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CreateBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub description: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCall {
    pub url: String,
    pub body: CreateBody,
}

// LiteLLM's transform_create_vector_store_request: an absent name takes the default and an explicit null drops the key
pub fn transform_create_request(
    optional_params: &BTreeMap<String, Json>,
    api_base: &str,
) -> Result<CreateCall, VertexRagStoreError> {
    let display_name = match optional_params.get("name") {
        None => Some(DEFAULT_CORPUS_NAME.to_owned()),
        Some(Json::Null) => None,
        Some(Json::Text(name)) => Some(name.clone()),
        Some(_) => return Err(VertexRagStoreError::ParamType("name", "a string")),
    };
    let labels = match optional_params.get("metadata") {
        None | Some(Json::Null) => None,
        Some(Json::Object(fields)) => Some(
            fields
                .iter()
                .map(|(name, value)| match value {
                    Json::Text(text) => Ok((name.clone(), text.clone())),
                    _ => Err(VertexRagStoreError::ParamType(
                        "metadata",
                        "an object of strings",
                    )),
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?,
        ),
        Some(_) => return Err(VertexRagStoreError::ParamType("metadata", "an object")),
    };
    let mut url = api_base.to_owned();
    url.push_str(CORPUS_SUFFIX);
    Ok(CreateCall {
        url,
        body: CreateBody {
            display_name,
            description: CORPUS_DESCRIPTION,
            labels,
        },
    })
}

// LiteLLM's transform_create_vector_store_response
pub fn transform_create_response(
    reply: &Reply<'_>,
    local: &LocalTimeContext,
) -> Result<VectorStoreCreateResponse, VertexRagStoreError> {
    match decode_vertex_rag_corpus(reply.body, local) {
        Ok(created) => Ok(created),
        Err(source) => Err(provider_error(reply, source)),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Corpus {
    name: Option<String>,
    display_name: Option<String>,
    #[serde(rename = "createTime")]
    create_time: Option<Json>,
    labels: Option<Json>,
}

pub fn decode_vertex_rag_corpus(
    body: &[u8],
    local: &LocalTimeContext,
) -> Result<VectorStoreCreateResponse, PayloadError> {
    let corpus: Corpus = parse(body)?;
    let name = corpus.name.unwrap_or_default();
    let id = name.rsplit('/').next().unwrap_or_default();
    Ok(serde_json::from_value(serde_json::json!({
        "id": id,
        "object": "vector_store",
        "created_at": corpus.create_time.as_ref().map_or(0, |value| create_time(value, local)),
        "name": corpus.display_name.unwrap_or_default(),
        "bytes": 0,
        "file_counts": {"in_progress": 0, "completed": 0, "failed": 0, "cancelled": 0, "total": 0},
        "status": "completed",
        "expires_after": null,
        "expires_at": null,
        "last_active_at": null,
        "metadata": labels(corpus.labels.as_ref())?,
    }))?)
}

// Python keeps a bool as an int, a float and any other type fall to zero, and an ISO string goes through B1384
fn create_time(value: &Json, local: &LocalTimeContext) -> i64 {
    match value {
        Json::Bool(flag) => i64::from(*flag),
        Json::Number(number) => number.as_i64().unwrap_or_default(),
        Json::Text(text) => created_at(Some(text), local),
        Json::Null | Json::List(_) | Json::Object(_) => 0,
    }
}

fn labels(value: Option<&Json>) -> Result<BTreeMap<String, String>, PayloadError> {
    let Some(Json::Object(fields)) = value else {
        return Ok(BTreeMap::new());
    };
    fields
        .iter()
        .map(|(name, value)| match value {
            Json::Text(text) => Ok((name.clone(), text.clone())),
            _ => Err(PayloadError::Field("labels", "an object of strings")),
        })
        .collect()
}
