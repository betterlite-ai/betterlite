use std::collections::BTreeMap;

use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::hosted_vllm_media::rerank as rerank_config;
use betterlite_openai::hosted_vllm_media::rerank::reply::transform_rerank_response;
use betterlite_openai::hosted_vllm_media::rerank::{
    CohereRerankCall, HostedVllmRerankError, MappedRerankParams, get_complete_url,
    map_cohere_rerank_params, transform_rerank_request, validate_environment,
};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::together_ai_rerank::ReturnDocuments;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::invalid;
use betterlite_oracle_helper_api::error::HelperError;

// LiteLLM's transform_rerank_response sees the status of the reply the case describes
const REPLY_STATUS: u16 = 200;
// LiteLLM raises UnsupportedParamsError with status 400 from a truncation that does not validate
const UNSUPPORTED_PARAMS_STATUS: u16 = 400;

#[derive(Debug, Deserialize)]
pub(crate) struct ParamsCall {
    api_base: Option<String>,
    env: BTreeMap<String, String>,
    headers: Headers,
    api_key: Option<String>,
    model: String,
    query: String,
    documents: Vec<RerankDocument>,
    top_n: Option<i64>,
    rank_fields: Option<Vec<String>>,
    return_documents: Option<bool>,
    max_chunks_per_doc: Option<i64>,
    max_tokens_per_doc: Option<i64>,
    instruction: Option<String>,
    non_default_params: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RequestCall {
    model: String,
    mapped: MappedRerankParams,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReplyCall {
    body: String,
}

pub(super) fn params_value(call: &ParamsCall) -> Result<Value, Box<HelperError>> {
    let key = call.api_key.clone().map(SecretString::from);
    let url = get_complete_url(call.api_base.as_deref()).map_err(rerank_failure)?;
    let headers = validate_environment(&call.headers, key.as_ref(), &call.env);
    let mapped = map_cohere_rerank_params(&rerank_call(call)).map_err(rerank_failure)?;
    // ast-grep-ignore: no-long-names
    let supported = rerank_config::get_supported_cohere_rerank_params();
    Ok(json!({
        "supported": supported,
        "url": url,
        "headers": headers,
        "mapped": mapped,
    }))
}

pub(super) fn request_value(call: &RequestCall) -> Result<Value, Box<HelperError>> {
    transform_rerank_request(&call.model, &call.mapped)
        .map(Value::Object)
        .map_err(rerank_failure)
}

pub(super) fn reply_value(call: &ReplyCall) -> Result<Value, Box<HelperError>> {
    let response = transform_rerank_response(REPLY_STATUS, call.body.as_bytes(), &Uuid::new_v4)
        .map_err(rerank_failure)?;
    serde_json::to_value(response).map_err(|error| invalid(error.to_string()))
}

fn rerank_call(call: &ParamsCall) -> CohereRerankCall<'_> {
    CohereRerankCall {
        model: &call.model,
        query: &call.query,
        documents: &call.documents,
        top_n: call.top_n,
        rank_fields: call.rank_fields.as_deref(),
        return_documents: call
            .return_documents
            .map_or(ReturnDocuments::Null, ReturnDocuments::Value),
        max_chunks_per_doc: call.max_chunks_per_doc,
        max_tokens_per_doc: call.max_tokens_per_doc,
        instruction: call.instruction.as_deref(),
        non_default_params: &call.non_default_params,
    }
}

fn rerank_failure(error: HostedVllmRerankError) -> Box<HelperError> {
    match error {
        HostedVllmRerankError::Truncation(source) => Box::new(HelperError {
            class: String::from("UnsupportedParamsError"),
            status: Some(UNSUPPORTED_PARAMS_STATUS),
            error_type: None,
            param: None,
            code: None,
            message: format!("hosted_vllm rerank: {source}"),
        }),
        other @ (HostedVllmRerankError::MissingApiBase
        | HostedVllmRerankError::MaxChunksPerDoc
        | HostedVllmRerankError::Parse { .. }
        | HostedVllmRerankError::NoResults { .. }
        | HostedVllmRerankError::MissingResultFields { .. }
        | HostedVllmRerankError::Shape(..)
        | HostedVllmRerankError::Encode(..)
        | HostedVllmRerankError::Score(..)) => invalid(other.to_string()),
    }
}
