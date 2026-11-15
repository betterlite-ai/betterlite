mod answers;
mod cost;

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::fireworks_ai_common::SessionParams;
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvKeys {
    #[serde(default, rename = "fireworks_api_key")]
    fireworks: Option<String>,
    #[serde(default, rename = "fireworks_ai_api_key")]
    ai: Option<String>,
    #[serde(default, rename = "fireworksai_api_key")]
    joined: Option<String>,
    #[serde(default, rename = "fireworks_ai_token")]
    token: Option<String>,
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
            "api_keys": answers::api_keys_answer(&input.api_keys),
            "sessions": answers::sessions_answer(&input.sessions),
            "affinity": answers::affinity_answer(&input.affinity),
            "mixin_headers": answers::collect(&input.mixin_headers, answers::mixin_answer)?,
            "embedding_params": answers::embedding_answer(&input.embedding_params),
            "pricing": answers::pricing_answer(&input.pricing),
            "costs": answers::cost_answers(&input.model_cost, &input.costs)?,
            "rerank_urls": answers::rerank_urls_answer(&input.rerank_urls),
            "rerank_headers": answers::collect(&input.rerank_headers, answers::rerank_headers_answer)?,
            "rerank_requests": answers::rerank_requests_answer(&input.rerank_requests)?,
            "rerank_responses": answers::collect(&input.rerank_responses, answers::response_answer)?,
        }))
    }
}
