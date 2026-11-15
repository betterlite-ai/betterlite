pub mod error;
pub mod response;
mod tests;
pub mod wire;

use secrecy::{ExposeSecret as _, SecretString};

use crate::api::rerank_contracts::records::RerankDocument;
use crate::fireworks_ai_common::{
    AUTHORIZATION_HEADER, CONTENT_TYPE_HEADER, FireworksEnv, resolve_api_key,
};
use crate::openai_base_config::Headers;
use error::FireworksRerankError;
use wire::OptionalRerankParams;

pub const DEFAULT_RERANK_URL: &str = "https://api.fireworks.ai/inference/v1/rerank";
pub const SUPPORTED_COHERE_RERANK_PARAMS: [&str; 4] =
    ["query", "documents", "top_n", "return_documents"];
pub const DEFAULT_RETURN_DOCUMENTS: bool = true;
const JSON_CONTENT_TYPE: &str = "application/json";
const RERANK_SUFFIX: &str = "/rerank";

#[must_use]
pub fn get_complete_url(api_base: Option<&str>) -> String {
    let Some(base) = api_base.filter(|base| !base.is_empty()) else {
        return DEFAULT_RERANK_URL.to_owned();
    };
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with(RERANK_SUFFIX) {
        return trimmed.to_owned();
    }
    if trimmed.ends_with("/v1") || trimmed.ends_with("/inference/v1") {
        return format!("{trimmed}{RERANK_SUFFIX}");
    }
    format!("{trimmed}/inference/v1{RERANK_SUFFIX}")
}
pub struct CohereRerankCall<'call> {
    pub query: &'call str,
    pub documents: &'call [RerankDocument],
    pub top_n: Option<i64>,
    pub return_documents: Option<bool>,
    pub rank_fields: Option<&'call [String]>,
    pub max_chunks_per_doc: Option<i64>,
    pub max_tokens_per_doc: Option<i64>,
    pub instruction: Option<&'call str>,
}

#[must_use]
pub fn map_cohere_rerank_params(call: &CohereRerankCall<'_>) -> OptionalRerankParams {
    OptionalRerankParams {
        query: Some(call.query.to_owned()),
        documents: Some(call.documents.to_vec()),
        top_n: call.top_n,
        return_documents: call.return_documents,
    }
}
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    env: &FireworksEnv,
) -> Result<Headers, FireworksRerankError> {
    let key = resolve_api_key(api_key, env).ok_or(FireworksRerankError::MissingApiKey)?;
    let mut merged = Headers::new();
    merged.insert(
        AUTHORIZATION_HEADER.to_owned(),
        format!("Bearer {}", key.expose_secret()),
    );
    merged.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
    merged.extend(headers.clone());
    Ok(merged)
}
