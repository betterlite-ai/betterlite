mod cost;

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::fireworks_ai_common::error::MissingApiKey;
use betterlite_openai::fireworks_ai_common::{
    FireworksEnv, SessionParams, get_session_id, resolve_api_key, validate_environment,
    with_session_affinity,
};
use betterlite_openai::fireworks_ai_cost::get_base_model_for_pricing;
use betterlite_openai::fireworks_ai_embedding::{map_openai_params, supported_openai_params};
use betterlite_openai::fireworks_ai_rerank::error::FireworksRerankError;
use betterlite_openai::fireworks_ai_rerank::response::{UuidIds, transform_rerank_response};
use betterlite_openai::fireworks_ai_rerank::wire::{
    OptionalRerankParams, transform_rerank_request,
};
use betterlite_openai::fireworks_ai_rerank::{
    SUPPORTED_COHERE_RERANK_PARAMS, get_complete_url,
    validate_environment as validate_rerank_environment,
};
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
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
            "api_keys": collect(&input.api_keys, api_key_answer)?,
            "sessions": collect(&input.sessions, session_answer)?,
            "affinity": collect(&input.affinity, affinity_answer)?,
            "mixin_headers": collect(&input.mixin_headers, mixin_answer)?,
            "embedding_params": collect(&input.embedding_params, embedding_answer)?,
            "pricing": collect(&input.pricing, pricing_answer)?,
            "costs": cost_answers(&input.model_cost, &input.costs)?,
            "rerank_urls": collect(&input.rerank_urls, url_answer)?,
            "rerank_headers": collect(&input.rerank_headers, rerank_headers_answer)?,
            "rerank_requests": collect(&input.rerank_requests, request_answer)?,
            "rerank_responses": collect(&input.rerank_responses, response_answer)?,
        }))
    }
}

fn collect<T>(
    items: &[T],
    answer: fn(&T) -> Result<Value, Box<crate::helpers::HelperError>>,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let answers = items.iter().map(answer).collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(answers))
}

fn env_of(keys: &EnvKeys) -> FireworksEnv {
    FireworksEnv {
        fireworks_api_key: keys.fireworks_api_key.as_deref().map(SecretString::from),
        fireworks_ai_api_key: keys.fireworks_ai_api_key.as_deref().map(SecretString::from),
        fireworksai_api_key: keys.fireworksai_api_key.as_deref().map(SecretString::from),
        fireworks_ai_token: keys.fireworks_ai_token.as_deref().map(SecretString::from),
    }
}

fn api_key_answer(call: &ApiKeyCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let argument = call.argument.as_deref().map(SecretString::from);
    let resolved = resolve_api_key(argument.as_ref(), &env_of(&call.env));
    Ok(json!(resolved.map(|key| key.expose_secret().clone())))
}

fn session_answer(call: &SessionCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    Ok(json!(get_session_id(&call.litellm_params)))
}

fn affinity_answer(call: &AffinityCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let pinned = with_session_affinity(&call.headers, &call.litellm_params);
    Ok(json!(pinned))
}

fn mixin_answer(call: &MixinCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let argument = call.api_key.as_deref().map(SecretString::from);
    match validate_environment(
        &call.headers,
        argument.as_ref(),
        &env_of(&call.env),
        &call.litellm_params,
    ) {
        Ok(headers) => Ok(json!(headers)),
        Err(MissingApiKey) => Err(Box::new(crate::helpers::HelperError {
            class: String::from("MissingApiKey"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: String::from("FIREWORKS_API_KEY is not set"),
        })),
    }
}

fn embedding_answer(call: &EmbeddingCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let supported = supported_openai_params(&call.model);
    let mapped = map_openai_params(
        &call.non_default_params,
        call.optional_params.clone(),
        &call.model,
    );
    Ok(json!({
        "supported": supported,
        "mapped": mapped,
    }))
}

fn pricing_answer(call: &PricingCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    Ok(json!(get_base_model_for_pricing(
        &call.model,
        &betterlite_openai::fireworks_ai_cost::FireworksPricingTiers::default(),
    )))
}

fn cost_answers(
    model_cost: &BTreeMap<String, Value>,
    calls: &[cost::CostCall],
) -> Result<Value, Box<crate::helpers::HelperError>> {
    if calls.is_empty() {
        return Ok(Value::Array(Vec::new()));
    }
    let lookup = cost::ModelCostLookup::from_map(model_cost).ok_or_else(|| {
        Box::new(crate::helpers::HelperError {
            class: String::from("InvalidModelCost"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: String::from("the model cost map is not a JSON object of model entries"),
        })
    })?;
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let answer = cost::answer(call, &lookup)?;
        answers.push(json!({
            "prompt_cost": answer.prompt_cost,
            "completion_cost": answer.completion_cost,
        }));
    }
    Ok(Value::Array(answers))
}

fn url_answer(call: &UrlCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    Ok(json!(get_complete_url(call.api_base.as_deref())))
}

fn rerank_headers_answer(call: &HeadersCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let argument = call.api_key.as_deref().map(SecretString::from);
    let headers = validate_rerank_environment(&call.headers, argument.as_ref(), &env_of(&call.env))
        .map_err(rerank_failure)?;
    Ok(json!(headers))
}

fn request_answer(call: &RequestCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let params = OptionalRerankParams {
        query: call.params.query.clone(),
        documents: call.params.documents.clone(),
        top_n: call.params.top_n,
        return_documents: call.params.return_documents,
    };
    let body = transform_rerank_request(&call.model, &params).map_err(rerank_failure)?;
    Ok(json!({
        "supported": SUPPORTED_COHERE_RERANK_PARAMS,
        "body": body,
    }))
}

fn response_answer(call: &ResponseCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let response =
        transform_rerank_response(call.status, &call.headers, call.body.as_bytes(), &UuidIds)
            .map_err(rerank_failure)?;
    serde_json::to_value(response).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: String::from("SerializationError"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

fn rerank_failure(error: FireworksRerankError) -> Box<crate::helpers::HelperError> {
    let (class, status) = match &error {
        FireworksRerankError::Fireworks(exception) => {
            ("FireworksError", Some(exception.status_code))
        }
        FireworksRerankError::MissingApiKey => ("MissingApiKey", None),
        FireworksRerankError::MissingQuery => ("MissingQuery", None),
        FireworksRerankError::MissingDocuments => ("MissingDocuments", None),
        FireworksRerankError::ResponseTooLarge { .. } => ("ResponseTooLarge", None),
        FireworksRerankError::ResponseDecode { .. } => ("ResponseDecode", None),
        FireworksRerankError::NoResults => ("NoResults", None),
        FireworksRerankError::MissingResultField { .. } => ("MissingResultField", None),
        FireworksRerankError::IndexShape => ("IndexShape", None),
        FireworksRerankError::RelevanceScore { .. } => ("RelevanceScore", None),
    };
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
