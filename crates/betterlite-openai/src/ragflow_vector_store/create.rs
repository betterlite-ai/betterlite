use serde::Serialize;
use serde_json::Value;

use crate::vector_stores::Request;
use crate::vector_stores::store::CreateParams;

use super::error::RagflowError;

const DEFAULT_CHUNK_METHOD: &str = "naive";

#[derive(Serialize)]
struct CreateBody<'request> {
    name: &'request str,
    #[serde(skip_serializing_if = "Option::is_none")]
    avatar: Option<&'request Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'request Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    embedding_model: Option<&'request Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    permission: Option<&'request Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chunk_method: Option<&'request Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parser_config: Option<&'request Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_type: Option<&'request Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pipeline_id: Option<&'request Value>,
}

pub fn create_request(params: &CreateParams, api_base: &str) -> Result<Request, RagflowError> {
    let name = params
        .name
        .as_deref()
        .filter(|name| !name.is_empty())
        .ok_or(RagflowError::MissingName)?;
    let metadata = params
        .metadata
        .as_ref()
        .filter(|metadata| !metadata.is_empty());
    let field = |key: &str| {
        let values = metadata?;
        values.get(key)
    };
    let mut body = CreateBody {
        name,
        avatar: field("avatar"),
        description: field("description"),
        embedding_model: field("embedding_model"),
        permission: field("permission"),
        chunk_method: field("chunk_method"),
        parser_config: field("parser_config"),
        parse_type: field("parse_type"),
        pipeline_id: field("pipeline_id"),
    };
    if body.chunk_method.is_some() && body.pipeline_id.is_some() {
        return Err(RagflowError::ChunkMethodExclusive);
    }
    let naive = Value::String(DEFAULT_CHUNK_METHOD.to_owned());
    if body.chunk_method.is_none() && body.pipeline_id.is_none() {
        body.chunk_method = Some(&naive);
    }
    Ok(Request {
        url: api_base.to_owned(),
        body: serde_json::json!(body),
    })
}
