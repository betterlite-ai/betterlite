use serde::Serialize;

use super::error::FireworksRerankError;
use crate::api::rerank_contracts::records::RerankDocument;

pub const FIREWORKS_AI_PREFIX: &str = "fireworks_ai/";
pub const FIREWORKS_PREFIX: &str = "fireworks/";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OptionalRerankParams {
    pub query: Option<String>,
    pub documents: Option<Vec<RerankDocument>>,
    pub top_n: Option<i64>,
    pub return_documents: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RerankRequestBody {
    pub model: String,
    pub query: String,
    pub documents: Vec<RerankDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_n: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_documents: Option<bool>,
}
pub fn transform_rerank_request(
    model: &str,
    params: &OptionalRerankParams,
) -> Result<RerankRequestBody, FireworksRerankError> {
    let query = params
        .query
        .clone()
        .ok_or(FireworksRerankError::MissingQuery)?;
    let documents = params
        .documents
        .clone()
        .ok_or(FireworksRerankError::MissingDocuments)?;
    let stripped = if model.starts_with(FIREWORKS_AI_PREFIX) {
        model.replace(FIREWORKS_AI_PREFIX, "")
    } else {
        model.to_owned()
    };
    let rendered = if stripped.starts_with(FIREWORKS_PREFIX) {
        stripped
    } else {
        format!("{FIREWORKS_PREFIX}{stripped}")
    };
    Ok(RerankRequestBody {
        model: rendered,
        query,
        documents,
        top_n: params.top_n,
        return_documents: params.return_documents,
    })
}
