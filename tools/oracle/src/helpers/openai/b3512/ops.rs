use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::ir;
use betterlite_models::decisions::{DecisionsJson, Extra};
use betterlite_openai::decisions::transformation::provider_config::{
    DecisionsProviderConfig as _, ParseResponseError, SigningContext,
};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::providers::strands_decider::StrandsDeciderDecisionsConfig;
use betterlite_openai::providers::strands_decider::arn::{agentcore_runtime, invoked_runtime};
use betterlite_openai::providers::strands_decider::headers::runtime_headers;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::input::{RuntimeStep, SignStep, Step, TransformStep};
use crate::helpers::{HelperError, HelperResult};

pub(super) fn execute(step: &Step) -> HelperResult {
    match step {
        Step::Runtime(case) => Ok(runtime_step(case)),
        Step::Config => Ok(config_step()),
        Step::Sign(case) => sign_step(case),
        Step::Transform(case) => transform_step(case),
    }
}

fn runtime_step(case: &RuntimeStep) -> Value {
    let config = StrandsDeciderDecisionsConfig::default();
    let runtime = agentcore_runtime(&case.api_base);
    let complete_url = config.get_complete_url(&case.api_base, "m");
    let invoked = invoked_runtime(&complete_url);
    json!({
        "runtime": runtime.map(|parsed| json!({
            "arn": parsed.arn,
            "region": parsed.region,
            "url": parsed.invocations_url(),
            "default_session_id": parsed.default_session_id(),
        })),
        "signs_with_aws": config.signs_with_aws(&case.api_base),
        "complete_url": complete_url,
        "invoked_region": invoked.map(|called| called.region),
    })
}

fn config_step() -> Value {
    let config = StrandsDeciderDecisionsConfig::default();
    json!({
        "api_key_env": StrandsDeciderDecisionsConfig::API_KEY_ENV,
        "api_base_env": StrandsDeciderDecisionsConfig::API_BASE_ENV,
        "api_key_required": StrandsDeciderDecisionsConfig::API_KEY_REQUIRED,
        "api_key": config.resolve_api_key(None, |_| String::new()),
        "api_base": config.resolve_api_base(None, |_| String::new()),
    })
}

fn sign_step(case: &SignStep) -> HelperResult {
    let now = epoch(case.now)?;
    let config = StrandsDeciderDecisionsConfig {
        credentials: None,
        clock: Arc::new(move || now),
    };
    let request = ir_request(&case.state, &case.question, &case.instructions);
    let wire = config
        .transform_decisions_request(&case.model, &request, "strands_decider")
        .map_err(|error| helper_error(&error.to_string()))?;
    let url = config.get_complete_url(&case.api_base, &case.model);
    let api_key = case.api_key.as_deref().map(SecretString::from);
    let headers: Headers = case
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let dropped = agentcore_runtime(&case.api_base).map_or_else(Vec::new, |runtime| {
        runtime_headers(&headers, &runtime, &case.litellm_params).dropped
    });
    let context = SigningContext {
        url: &url,
        api_base: &case.api_base,
        body: &wire,
        api_key: api_key.as_ref(),
        litellm_params: &case.litellm_params,
    };
    let (signed, payload) = config.sign_request(headers, context);
    Ok(json!({
        "dropped": dropped,
        "headers": signed,
        "body": payload.map(|bytes| STANDARD.encode(bytes)),
    }))
}

fn transform_step(case: &TransformStep) -> HelperResult {
    let config = StrandsDeciderDecisionsConfig::default();
    let request = ir_request(&case.state, &case.question, &case.instructions);
    match config.transform_decisions_response(
        &case.model,
        "strands_decider",
        case.body.as_bytes(),
        &request,
    ) {
        Ok(response) => Ok(project_response(&response)),
        Err(error) => Err(Box::new(HelperError {
            class: "BaseLLMException".to_owned(),
            status: response_status(&error),
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: error.to_string(),
        })),
    }
}

fn project_response(response: &ir::DecisionResponse) -> Value {
    json!({
        "model": response.model,
        "answers": response.answers.iter().map(answer_projection).collect::<Vec<Value>>(),
        "usage": {
            "input_tokens": response.usage.input_tokens.0,
            "output_tokens": response.usage.output_tokens.0,
            "cached_tokens": response.usage.cached_tokens.0,
            "cache_write_tokens": response.usage.cache_write_tokens.0,
        },
    })
}

fn answer_projection(answer: &ir::DecisionAnswer) -> Value {
    match answer {
        ir::DecisionAnswer::Predicate { probability, .. } => json!({
            "kind": "DecisionsIRPredicateAnswer",
            "probability": probability,
        }),
        ir::DecisionAnswer::Choice { .. } => json!({"kind": "DecisionsIRChoiceAnswer"}),
        ir::DecisionAnswer::Score { .. } => json!({"kind": "DecisionsIRScoreAnswer"}),
        ir::DecisionAnswer::Refusal => json!({"kind": "DecisionsIRRefusal"}),
    }
}

fn response_status(error: &ParseResponseError) -> Option<u16> {
    match error {
        ParseResponseError::Invalid(provider) => Some(provider.status_code),
        ParseResponseError::Malformed(_) => None,
    }
}

fn ir_request(state: &str, question: &str, instructions: &str) -> ir::DecisionRequest {
    ir::DecisionRequest {
        input: ir::DecisionInput::State(DecisionsJson::Text(state.to_owned())),
        questions: vec![ir::DecisionQuestion::Predicate(ir::PredicateQuestion {
            name: Some(question.to_owned()),
            instructions: Some(DecisionsJson::Text(instructions.to_owned())),
            criteria: None,
            extra: Extra::default(),
        })],
        safety_identifier: None,
    }
}

fn epoch(seconds: u64) -> Result<SystemTime, Box<HelperError>> {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .ok_or_else(|| helper_error("the signing time overflows SystemTime"))
}

fn helper_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "StrandsDeciderHelper".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: message.to_owned(),
    })
}
