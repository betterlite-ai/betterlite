use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::api::rerank_contracts::records::{RerankDocument, RerankRequest, TruncationSide};
use crate::error::{ProviderError, ProviderErrorParts};
use crate::openai_base_config::Headers;
use crate::together_ai_rerank::ReturnDocuments;

use super::resolved_api_key;

pub mod reply;
mod tests;

const AUTHORIZATION: &str = "Authorization";
const JSON_CONTENT_TYPE: &str = "application/json";

// LiteLLM advertises exactly these ten Cohere-style parameters and drops every other request key
#[rustfmt::skip]
pub const COHERE_RERANK_PARAMS: [&str; 10] = [
    "query", "documents", "top_n", "rank_fields", "return_documents", "max_tokens_per_doc",
    "instruction", "truncate_prompt_tokens", "truncation_side", "max_tokens_per_query",
];

#[derive(Debug, Error)]
pub enum HostedVllmRerankError {
    #[error("api_base must be provided for Hosted VLLM rerank")]
    MissingApiBase,
    #[error("Hosted VLLM does not support max_chunks_per_doc")]
    MaxChunksPerDoc,
    #[error("hosted_vllm rerank: {0}")]
    Truncation(#[source] serde_json::Error),
    #[error("Error parsing response: {text}, status_code={status_code}")]
    Parse { text: String, status_code: u16 },
    #[error("No results found in the response={response}")]
    NoResults { response: Value },
    #[error("Missing required fields in the result={result}")]
    MissingResultFields { result: Value },
    #[error("the rerank reply could not be decoded")]
    Shape(#[source] serde_json::Error),
    #[error("the rerank record cannot be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("score cannot be represented as an exact decimal")]
    Score(#[source] DecimalLexemeError),
}

// LiteLLM's HostedVLLMRerankTruncationParams: the four keys are validated, every other key is ignored
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct TruncationParams {
    pub truncate_prompt_tokens: Option<i64>,
    pub truncation_side: Option<TruncationSide>,
    pub max_tokens_per_query: Option<i64>,
    pub max_tokens_per_doc: Option<i64>,
}

// LiteLLM's OptionalRerankParams as map_cohere_rerank_params returns it: every key present, nulls included
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappedRerankParams {
    pub query: String,
    pub documents: Vec<RerankDocument>,
    #[serde(default)]
    pub top_n: Option<i64>,
    #[serde(default)]
    pub rank_fields: Option<Vec<String>>,
    #[serde(default)]
    pub return_documents: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default)]
    pub max_tokens_per_doc: Option<i64>,
    #[serde(default)]
    pub truncate_prompt_tokens: Option<i64>,
    #[serde(default)]
    pub truncation_side: Option<TruncationSide>,
    #[serde(default)]
    pub max_tokens_per_query: Option<i64>,
}

pub struct CohereRerankCall<'request> {
    pub model: &'request str,
    pub query: &'request str,
    pub documents: &'request [RerankDocument],
    pub top_n: Option<i64>,
    pub rank_fields: Option<&'request [String]>,
    pub return_documents: ReturnDocuments,
    pub max_chunks_per_doc: Option<i64>,
    pub max_tokens_per_doc: Option<i64>,
    pub instruction: Option<&'request str>,
    pub non_default_params: &'request Map<String, Value>,
}

pub fn validated_truncation_params(
    non_default_params: &Map<String, Value>,
) -> Result<TruncationParams, HostedVllmRerankError> {
    serde_json::from_value(Value::Object(non_default_params.clone()))
        .map_err(HostedVllmRerankError::Truncation)
}

pub fn map_cohere_rerank_params(
    call: &CohereRerankCall<'_>,
) -> Result<MappedRerankParams, HostedVllmRerankError> {
    if call.max_chunks_per_doc.is_some() {
        return Err(HostedVllmRerankError::MaxChunksPerDoc);
    }
    let truncation = validated_truncation_params(call.non_default_params)?;
    Ok(MappedRerankParams {
        query: call.query.to_owned(),
        documents: call.documents.to_vec(),
        top_n: call.top_n,
        rank_fields: call.rank_fields.map(<[String]>::to_vec),
        return_documents: match call.return_documents {
            ReturnDocuments::Default => Some(true),
            ReturnDocuments::Null => None,
            ReturnDocuments::Value(flag) => Some(flag),
        },
        instruction: call.instruction.map(str::to_owned),
        max_tokens_per_doc: call.max_tokens_per_doc,
        truncate_prompt_tokens: truncation.truncate_prompt_tokens,
        truncation_side: truncation.truncation_side,
        max_tokens_per_query: truncation.max_tokens_per_query,
    })
}

#[must_use]
// ast-grep-ignore: no-long-names, no-one-line-functions
pub fn get_supported_cohere_rerank_params() -> Vec<&'static str> {
    COHERE_RERANK_PARAMS.to_vec()
}

pub fn get_complete_url(api_base: Option<&str>) -> Result<String, HostedVllmRerankError> {
    let base = api_base
        .filter(|base| !base.is_empty())
        .ok_or(HostedVllmRerankError::MissingApiBase)?;
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/v1/rerank") {
        Ok(trimmed.replace("/v1/rerank", "/rerank"))
    } else if trimmed.ends_with("/rerank") {
        Ok(trimmed.to_owned())
    } else {
        Ok(format!("{trimmed}/rerank"))
    }
}

// LiteLLM keeps an explicitly empty key and reads the environment only when the key is absent
#[must_use]
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    environment: &dyn EnvironmentLookup,
) -> Headers {
    let key = api_key.map_or_else(
        || resolved_api_key(None, environment),
        |key| key.expose_secret().to_owned(),
    );
    let mut merged = Headers::new();
    merged.insert(String::from(AUTHORIZATION), format!("Bearer {key}"));
    merged.insert(String::from("accept"), String::from(JSON_CONTENT_TYPE));
    merged.insert(
        String::from("content-type"),
        String::from(JSON_CONTENT_TYPE),
    );
    // the caller's headers win on an exact key, and a differently cased one is a second entry
    merged.extend(headers.clone());
    merged
}

pub fn transform_rerank_request(
    model: &str,
    mapped: &MappedRerankParams,
) -> Result<Map<String, Value>, HostedVllmRerankError> {
    let request = RerankRequest {
        model: model.to_owned(),
        query: mapped.query.clone(),
        top_n: mapped.top_n,
        documents: mapped.documents.clone(),
        rank_fields: mapped.rank_fields.clone(),
        return_documents: mapped.return_documents,
        max_chunks_per_doc: None,
        max_tokens_per_doc: mapped.max_tokens_per_doc,
        instruction: mapped.instruction.clone(),
        truncate_prompt_tokens: mapped.truncate_prompt_tokens,
        truncation_side: mapped.truncation_side,
        max_tokens_per_query: mapped.max_tokens_per_query,
    };
    let encoded = serde_json::to_value(&request).map_err(HostedVllmRerankError::Encode)?;
    let mut body = serde_json::from_value::<Map<String, Value>>(encoded)
        .map_err(HostedVllmRerankError::Encode)?;
    body.retain(|_, value| !value.is_null());
    Ok(body)
}

#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code,
        message: message.to_owned(),
        headers: Some(headers.clone().into_iter().collect()),
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })
}
