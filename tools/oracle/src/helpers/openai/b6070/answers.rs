use std::collections::BTreeMap;

use betterlite_openai::fireworks_ai_common::error::MissingApiKey;
use betterlite_openai::fireworks_ai_common::{
    FireworksEnv, get_session_id, resolve_api_key, validate_environment, with_session_affinity,
};
use betterlite_openai::fireworks_ai_cost::get_base_model_for_pricing;
use betterlite_openai::fireworks_ai_embedding::{map_openai_params, supported_openai_params};
use betterlite_openai::fireworks_ai_rerank::error::FireworksRerankError;
use betterlite_openai::fireworks_ai_rerank::response::{UuidIds, transform_rerank_response};
use betterlite_openai::fireworks_ai_rerank::wire::{
    OptionalRerankParams, transform_rerank_request,
};
use betterlite_openai::fireworks_ai_rerank::{
    get_complete_url, validate_environment as validate_rerank_environment,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::{
    AffinityCall, ApiKeyCall, EmbeddingCall, HeadersCall, MixinCall, PricingCall, RequestCall,
    ResponseCall, SessionCall, UrlCall,
};
use crate::helpers::HelperError;

pub(super) fn collect<T>(
    items: &[T],
    answer: fn(&T) -> Result<Value, Box<HelperError>>,
) -> Result<Value, Box<HelperError>> {
    let answers = items.iter().map(answer).collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(answers))
}

pub(super) fn env_of(keys: &super::EnvKeys) -> FireworksEnv {
    FireworksEnv {
        fireworks_api_key: keys.fireworks_api_key.as_deref().map(SecretString::from),
        fireworks_ai_api_key: keys.fireworks_ai_api_key.as_deref().map(SecretString::from),
        fireworksai_api_key: keys.fireworksai_api_key.as_deref().map(SecretString::from),
        fireworks_ai_token: keys.fireworks_ai_token.as_deref().map(SecretString::from),
    }
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn api_key_answer(call: &ApiKeyCall) -> Result<Value, Box<HelperError>> {
    let argument = call.argument.as_deref().map(SecretString::from);
    let resolved = resolve_api_key(argument.as_ref(), &env_of(&call.env));
    Ok(json!(resolved.map(|key| key.expose_secret().clone())))
}

pub(super) fn session_answer(call: &SessionCall) -> Result<Value, Box<HelperError>> {
    Ok(json!(get_session_id(&call.litellm_params)))
}

pub(super) fn affinity_answer(call: &AffinityCall) -> Result<Value, Box<HelperError>> {
    let pinned = with_session_affinity(&call.headers, &call.litellm_params);
    Ok(json!(pinned))
}

pub(super) fn mixin_answer(call: &MixinCall) -> Result<Value, Box<HelperError>> {
    let argument = call.api_key.as_deref().map(SecretString::from);
    match validate_environment(
        &call.headers,
        argument.as_ref(),
        &env_of(&call.env),
        &call.litellm_params,
    ) {
        Ok(headers) => Ok(json!(headers)),
        Err(MissingApiKey) => Err(helper_error(
            "MissingApiKey",
            String::from("FIREWORKS_API_KEY is not set"),
        )),
    }
}

pub(super) fn embedding_answer(call: &EmbeddingCall) -> Result<Value, Box<HelperError>> {
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

pub(super) fn pricing_answer(call: &PricingCall) -> Result<Value, Box<HelperError>> {
    let tiers = betterlite_openai::fireworks_ai_cost::FireworksPricingTiers::default();
    Ok(json!(get_base_model_for_pricing(&call.model, &tiers)))
}

pub(super) fn cost_answers(
    model_cost: &BTreeMap<String, Value>,
    calls: &[super::cost::CostCall],
) -> Result<Value, Box<HelperError>> {
    if calls.is_empty() {
        return Ok(Value::Array(Vec::new()));
    }
    let lookup = super::cost::ModelCostLookup::from_map(model_cost)
        .ok_or_else(|| helper_error("InvalidModelCost", String::from("no model cost map")))?;
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let (prompt_cost, completion_cost) = super::cost::answer(call, &lookup)?;
        answers.push(json!({
            "prompt_cost": prompt_cost,
            "completion_cost": completion_cost,
        }));
    }
    Ok(Value::Array(answers))
}

pub(super) fn url_answer(call: &UrlCall) -> Result<Value, Box<HelperError>> {
    Ok(json!(get_complete_url(call.api_base.as_deref())))
}

pub(super) fn rerank_headers_answer(call: &HeadersCall) -> Result<Value, Box<HelperError>> {
    let argument = call.api_key.as_deref().map(SecretString::from);
    let headers = validate_rerank_environment(&call.headers, argument.as_ref(), &env_of(&call.env))
        .map_err(rerank_failure)?;
    Ok(json!(headers))
}

pub(super) fn request_answer(call: &RequestCall) -> Result<Value, Box<HelperError>> {
    let params = OptionalRerankParams {
        query: call.params.query.clone(),
        documents: call.params.documents.clone(),
        top_n: call.params.top_n,
        return_documents: call.params.return_documents,
    };
    let body = transform_rerank_request(&call.model, &params).map_err(rerank_failure)?;
    serde_json::to_value(body)
        .map_err(|error| helper_error("SerializationError", error.to_string()))
}

pub(super) fn response_answer(call: &ResponseCall) -> Result<Value, Box<HelperError>> {
    let response =
        transform_rerank_response(call.status, &call.headers, call.body.as_bytes(), &UuidIds)
            .map_err(rerank_failure)?;
    serde_json::to_value(response)
        .map_err(|error| helper_error("SerializationError", error.to_string()))
}

fn rerank_failure(error: FireworksRerankError) -> Box<HelperError> {
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
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
