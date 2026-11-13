use std::collections::BTreeMap;

use betterlite_openai::fireworks_ai_common::error::MissingApiKey;
use betterlite_openai::fireworks_ai_common::{FireworksEnv, resolve_api_key, validate_environment};
use betterlite_openai::fireworks_ai_rerank::error::FireworksRerankError;
use betterlite_openai::fireworks_ai_rerank::response::{UuidIds, transform_rerank_response};
use betterlite_openai::fireworks_ai_rerank::validate_environment as validate_rerank_environment;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::{ApiKeyCall, HeadersCall, MixinCall, ResponseCall};
use crate::helpers::HelperError;

pub(super) fn collect<T>(
    items: &[T],
    answer: fn(&T) -> Result<Value, Box<HelperError>>,
) -> Result<Value, Box<HelperError>> {
    let mut answers = Vec::with_capacity(items.len());
    for item in items {
        answers.push(answer(item)?);
    }
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

pub(super) fn rerank_headers_answer(call: &HeadersCall) -> Result<Value, Box<HelperError>> {
    let argument = call.api_key.as_deref().map(SecretString::from);
    let headers = validate_rerank_environment(&call.headers, argument.as_ref(), &env_of(&call.env))
        .map_err(rerank_failure)?;
    Ok(json!(headers))
}

pub(super) fn response_answer(call: &ResponseCall) -> Result<Value, Box<HelperError>> {
    match transform_rerank_response(call.status, &call.headers, call.body.as_bytes(), &UuidIds) {
        Ok(response) => Ok(json!(response)),
        Err(error) => Err(rerank_failure(error)),
    }
}

pub(super) fn rerank_failure(error: FireworksRerankError) -> Box<HelperError> {
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
