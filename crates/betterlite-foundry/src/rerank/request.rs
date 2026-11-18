use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::cohere_rerank::{self, RerankParams, RerankVersion};
use serde_json::Number;

use super::error::FoundryRerankError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundryRerankRequest {
    pub model: String,
    pub query: String,
    pub documents: Vec<RerankDocument>,
    pub top_n: Option<i64>,
    pub rank_fields: Option<Vec<String>>,
    pub return_documents: Option<bool>,
    pub max_chunks_per_doc: Option<i64>,
}

// Foundry inherits Cohere's v1 params: absent options leave the key out of the body and return_documents defaults to true
pub fn transform_request(
    request: &FoundryRerankRequest,
) -> Result<RerankParams, FoundryRerankError> {
    let options = request_options(request);
    let mapped =
        cohere_rerank::map_rerank_params(RerankVersion::V1, &options, &RerankParams::new());
    Ok(cohere_rerank::transform_rerank_request(
        RerankVersion::V1,
        &request.model,
        &mapped,
    )?)
}

fn request_options(request: &FoundryRerankRequest) -> RerankParams {
    RerankParams::from([
        ("query".to_owned(), Json::Text(request.query.clone())),
        (
            "documents".to_owned(),
            Json::List(
                request
                    .documents
                    .iter()
                    .map(RerankDocument::as_json)
                    .collect(),
            ),
        ),
        (
            "top_n".to_owned(),
            request
                .top_n
                .map_or(Json::Null, |number| Json::Number(Number::from(number))),
        ),
        (
            "rank_fields".to_owned(),
            request.rank_fields.as_ref().map_or(Json::Null, |fields| {
                Json::List(fields.iter().cloned().map(Json::Text).collect())
            }),
        ),
        (
            "return_documents".to_owned(),
            request.return_documents.map_or(Json::Null, Json::Bool),
        ),
        (
            "max_chunks_per_doc".to_owned(),
            request
                .max_chunks_per_doc
                .map_or(Json::Null, |number| Json::Number(Number::from(number))),
        ),
    ])
}
