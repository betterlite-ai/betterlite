use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{
    RerankResponse, RerankResponseMeta, RerankResponseResult,
};
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;
use betterlite_openai::cohere_rerank;

use super::error::FoundryRerankError;

pub const MODEL_GROUP_PARAM: &str = "llm_provider-azureml-model-group";
pub const HIDDEN_MODEL_PARAM: &str = "model";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundryRerankReply {
    pub id: Option<String>,
    pub results: Option<Vec<RerankResponseResult>>,
    pub meta: Option<RerankResponseMeta>,
    pub hidden_params: BTreeMap<String, Json>,
}

// LiteLLM reads the group from the reply's hidden params, so the caller hands the collected map in
pub fn transform_reply(
    status: u16,
    body: &[u8],
    limits: WireDecodeLimits,
    hidden_params: BTreeMap<String, Json>,
) -> Result<FoundryRerankReply, FoundryRerankError> {
    let mut reply = cohere_rerank::transform_rerank_response(status, body, limits)?;
    reply.hidden_params = hidden_params;
    let model = match reply.hidden_params.get(MODEL_GROUP_PARAM) {
        None | Some(Json::Null) => Json::Null,
        Some(Json::Text(group)) => Json::Text(base_model(group).to_owned()),
        Some(other) => other.clone(),
    };
    reply
        .hidden_params
        .insert(HIDDEN_MODEL_PARAM.to_owned(), model);
    Ok(FoundryRerankReply {
        id: reply.id,
        results: reply.results,
        meta: reply.meta,
        hidden_params: reply.hidden_params,
    })
}

// LiteLLM writes the hidden model even when there is no group, so the key is always present
fn base_model(group: &str) -> &str {
    match group {
        "offer-cohere-rerank-mul-paygo" => "azure_ai/cohere-rerank-v3-multilingual",
        "offer-cohere-rerank-eng-paygo" => "azure_ai/cohere-rerank-v3-english",
        other => other,
    }
}
