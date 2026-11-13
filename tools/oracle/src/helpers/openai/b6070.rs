mod answers;
mod cost;

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::fireworks_ai_common::{
    SessionParams, get_session_id, with_session_affinity,
};
use betterlite_openai::fireworks_ai_cost::{FireworksPricingTiers, get_base_model_for_pricing};
use betterlite_openai::fireworks_ai_embedding::{map_openai_params, supported_openai_params};
use betterlite_openai::fireworks_ai_rerank::get_complete_url;
use betterlite_openai::fireworks_ai_rerank::wire::{
    OptionalRerankParams, transform_rerank_request,
};
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvKeys {
    #[serde(default)]
    fireworks_api_key: Option<String>,
    #[serde(default)]
    fireworks_ai_api_key: Option<String>,
    #[serde(default)]
    fireworksai_api_key: Option<String>,
    #[serde(default)]
    fireworks_ai_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApiKeyCall {
    argument: Option<String>,
    #[serde(default)]
    env: EnvKeys,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionCall {
    litellm_params: SessionParams,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AffinityCall {
    headers: Headers,
    litellm_params: SessionParams,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MixinCall {
    headers: Headers,
    api_key: Option<String>,
    #[serde(default)]
    env: EnvKeys,
    litellm_params: SessionParams,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbeddingCall {
    model: String,
    non_default_params: IndexMap<String, Json>,
    optional_params: IndexMap<String, Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PricingCall {
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlCall {
    api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadersCall {
    headers: Headers,
    api_key: Option<String>,
    #[serde(default)]
    env: EnvKeys,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ParamsInput {
    query: Option<String>,
    documents: Option<Vec<RerankDocument>>,
    top_n: Option<i64>,
    return_documents: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCall {
    model: String,
    params: ParamsInput,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseCall {
    status: u16,
    headers: Headers,
    body: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    model_cost: BTreeMap<String, Value>,
    api_keys: Vec<ApiKeyCall>,
    sessions: Vec<SessionCall>,
    affinity: Vec<AffinityCall>,
    mixin_headers: Vec<MixinCall>,
    embedding_params: Vec<EmbeddingCall>,
    pricing: Vec<PricingCall>,
    costs: Vec<cost::CostCall>,
    rerank_urls: Vec<UrlCall>,
    rerank_headers: Vec<HeadersCall>,
    rerank_requests: Vec<RequestCall>,
    rerank_responses: Vec<ResponseCall>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(json!({
            "api_keys": answers::collect(&input.api_keys, answers::api_key_answer)?,
            "sessions": input
                .sessions
                .iter()
                .map(|call| json!(get_session_id(&call.litellm_params)))
                .collect::<Vec<_>>(),
            "affinity": input
                .affinity
                .iter()
                .map(|call| json!(with_session_affinity(&call.headers, &call.litellm_params)))
                .collect::<Vec<_>>(),
            "mixin_headers": answers::collect(&input.mixin_headers, answers::mixin_answer)?,
            "embedding_params": input
                .embedding_params
                .iter()
                .map(|call| json!({
                    "supported": supported_openai_params(&call.model),
                    "mapped": map_openai_params(
                        &call.non_default_params,
                        call.optional_params.clone(),
                        &call.model,
                    ),
                }))
                .collect::<Vec<_>>(),
            "pricing": input
                .pricing
                .iter()
                .map(|call| json!(get_base_model_for_pricing(
                    &call.model,
                    &FireworksPricingTiers::default(),
                )))
                .collect::<Vec<_>>(),
            "costs": answers::cost_answers(&input.model_cost, &input.costs)?,
            "rerank_urls": input
                .rerank_urls
                .iter()
                .map(|call| json!(get_complete_url(call.api_base.as_deref())))
                .collect::<Vec<_>>(),
            "rerank_headers": answers::collect(&input.rerank_headers, answers::rerank_headers_answer)?,
            "rerank_requests": input
                .rerank_requests
                .iter()
                .map(|call| {
                    let params = OptionalRerankParams {
                        query: call.params.query.clone(),
                        documents: call.params.documents.clone(),
                        top_n: call.params.top_n,
                        return_documents: call.params.return_documents,
                    };
                    transform_rerank_request(&call.model, &params)
                        .map(|body| json!(body))
                        .map_err(answers::rerank_failure)
                })
                .collect::<Result<Vec<_>, _>>()?,
            "rerank_responses": answers::collect(&input.rerank_responses, answers::response_answer)?,
        }))
    }
}
