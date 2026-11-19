use std::collections::BTreeMap;

use betterlite_openai::edenai_common::{
    EdenAiEnv, reported_cost, reported_cost_from_bytes, reported_cost_from_headers,
};
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::SecretString;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::calls::{
    catalog_value, chat_params_value, chat_request_value, credentials_value, embedding_plan_value,
    messages_environment_value, prepare_value,
};
use super::input::Input;
use super::replies::{
    chat_reply_value, chunk_value, embedding_reply_value, messages_reply_value,
    responses_reply_value,
};

pub(super) const API_BASE_ENV: &str = "EDENAI_API_BASE";
pub(super) const API_KEY_ENV: &str = "EDENAI_API_KEY";
pub(super) const MODEL: &str = "m";

pub(super) fn run(input: &Input) -> HelperResult {
    if !input.costs.is_empty() {
        let steps: Vec<Option<f64>> = input
            .costs
            .iter()
            .map(|payload| reported_cost(payload).and_then(|value| value.to_f64()))
            .collect();
        return Ok(json!({"steps": steps}));
    }
    if !input.cost_bodies.is_empty() {
        let steps: Vec<Option<f64>> = input
            .cost_bodies
            .iter()
            .map(|text| reported_cost_from_bytes(text.as_bytes()).and_then(|value| value.to_f64()))
            .collect();
        return Ok(json!({"steps": steps}));
    }
    if !input.cost_headers.is_empty() {
        let steps: Vec<Option<f64>> = input
            .cost_headers
            .iter()
            .map(|headers| reported_cost_from_headers(headers).and_then(|value| value.to_f64()))
            .collect();
        return Ok(json!({"steps": steps}));
    }
    if !input.credentials.is_empty() {
        let steps = report(&input.credentials, credentials_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.chat_params.is_empty() {
        let steps = report(&input.chat_params, chat_params_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.chat_requests.is_empty() {
        let steps = report(&input.chat_requests, chat_request_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.chat_replies.is_empty() {
        let steps = report_text(&input.chat_replies, chat_reply_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.chunks.is_empty() {
        let steps = report(&input.chunks, chunk_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.models.is_empty() {
        let steps = report(&input.models, catalog_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.embedding_plans.is_empty() {
        let steps = report(&input.embedding_plans, embedding_plan_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.embedding_replies.is_empty() {
        let steps = report_text(&input.embedding_replies, embedding_reply_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.responses_prepare.is_empty() {
        let steps = report(&input.responses_prepare, prepare_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.responses_replies.is_empty() {
        let steps = report_text(&input.responses_replies, responses_reply_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.messages_environments.is_empty() {
        let steps = report(&input.messages_environments, messages_environment_value)?;
        return Ok(json!({"steps": steps}));
    }
    if !input.messages_replies.is_empty() {
        let steps = report_text(&input.messages_replies, messages_reply_value)?;
        return Ok(json!({"steps": steps}));
    }
    Err(Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: String::from("expected one b6100 variant"),
    }))
}

fn report<T>(
    cases: &[T],
    convert: fn(&T) -> Result<Value, Box<HelperError>>,
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut values = Vec::with_capacity(cases.len());
    for case in cases {
        values.push(convert(case)?);
    }
    Ok(values)
}

fn report_text(
    cases: &[String],
    convert: fn(&str) -> Result<Value, Box<HelperError>>,
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut values = Vec::with_capacity(cases.len());
    for case in cases {
        values.push(convert(case.as_str())?);
    }
    Ok(values)
}

// the caller fills api_key and api_base from the environment and litellm.api_key from the call
pub(super) fn env_with(environment: &BTreeMap<String, String>, global: Option<&str>) -> EdenAiEnv {
    EdenAiEnv {
        api_key: environment
            .get(API_KEY_ENV)
            .cloned()
            .map(SecretString::from),
        api_base: environment.get(API_BASE_ENV).cloned(),
        global_api_key: global.map(str::to_owned).map(SecretString::from),
    }
}
