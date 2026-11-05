use super::Request;
use super::error::VectorStoreError;
use crate::api::vector_store_records::records::vector::VectorStoreChunkingStrategy;
use crate::openai_base_config::metadata_filter_limit;
use betterlite_http::url_path::encode_segment;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Attributes {
    Map(IndexMap<String, Value>),
    Other(Value),
}

#[derive(Serialize)]
#[serde(untagged)]
enum Projected {
    Map(IndexMap<String, String>),
    Other(Value),
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FileCreateParams {
    pub file_id: Option<String>,
    pub attributes: Option<Attributes>,
    pub chunking_strategy: Option<VectorStoreChunkingStrategy>,
}

#[derive(Serialize)]
struct CreateBody<'request> {
    #[serde(skip_serializing_if = "Option::is_none")]
    file_id: &'request Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<Projected>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chunking_strategy: &'request Option<VectorStoreChunkingStrategy>,
}

#[derive(Serialize)]
struct UpdateBody {
    attributes: Option<Projected>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FileListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileUpdateParams {
    pub attributes: Attributes,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOperation {
    Retrieve,
    Content,
    Delete,
    Update(FileUpdateParams),
}

fn project(attributes: &Attributes) -> Option<Projected> {
    match attributes {
        Attributes::Map(map) => metadata_filter_limit(Some(map)).map(Projected::Map),
        Attributes::Other(value) => Some(Projected::Other(value.clone())),
    }
}

#[must_use]
pub fn create_request(params: &FileCreateParams, api_base: &str) -> Request {
    Request {
        url: api_base.to_owned(),
        body: json!(CreateBody {
            file_id: &params.file_id,
            attributes: params.attributes.as_ref().and_then(project),
            chunking_strategy: &params.chunking_strategy,
        }),
    }
}

#[must_use]
pub fn list_request(params: &FileListParams, api_base: &str) -> Request {
    Request {
        url: api_base.to_owned(),
        body: json!(params),
    }
}

pub fn file_request(
    operation: &FileOperation,
    file_id: &str,
    api_base: &str,
) -> Result<Request, VectorStoreError> {
    let encoded = encode_segment(file_id, "file_id")?;
    let (suffix, body) = match operation {
        FileOperation::Content => ("/content", Value::Object(Map::new())),
        FileOperation::Retrieve | FileOperation::Delete => ("", Value::Object(Map::new())),
        FileOperation::Update(params) => (
            "",
            json!(UpdateBody {
                attributes: project(&params.attributes),
            }),
        ),
    };
    Ok(Request {
        url: format!("{api_base}/{encoded}{suffix}"),
        body,
    })
}
