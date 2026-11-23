use betterlite_models::decisions::openai::DecisionResponse as OpenAiDecisionResponse;
use betterlite_models::decisions::{HiddenParams, Json, ir, systemone};
use betterlite_openai::decisions::call::DecisionsCall;
use betterlite_openai::decisions::openai::response::ResponseConversionError;
use rust_decimal::Decimal;
use serde_json::Number;

use super::support::*;
use crate::api::decisions_send::format::{DecisionsOutput, format_response};
use crate::helpers::RESPONSE_COST_HEADER;

fn answer_count(error: ResponseConversionError) -> (usize, usize) {
    match error {
        ResponseConversionError::AnswerCount { answers, questions } => (answers, questions),
        other @ (ResponseConversionError::LevelIndex
        | ResponseConversionError::Probability
        | ResponseConversionError::TokenCount
        | ResponseConversionError::Text(_)) => {
            panic!("expected an answer count error, got {other}")
        }
    }
}

fn openai_output(
    config: &TestConfig,
    call: &DecisionsCall<'_>,
    model: Option<&str>,
) -> OpenAiDecisionResponse {
    match format_response(config, call, &ir_response(model)).expect("the response formats") {
        DecisionsOutput::OpenAi(response) => response,
        DecisionsOutput::SystemOne(_) => {
            panic!("an OpenAI request answers with an OpenAI response")
        }
    }
}

fn cost_header(hidden: &HiddenParams) -> Option<f64> {
    let Some(Json::Object(entries)) = hidden.get("additional_headers") else {
        panic!("the cost header is set");
    };
    assert_eq!(entries.len(), 1);
    let Some(Json::Number(cost)) = entries.get(RESPONSE_COST_HEADER) else {
        panic!("the cost header holds a number");
    };
    cost.as_f64()
}

// format_response: the shape follows the request, and the hidden params follow the config's cost

#[test]
fn format_response_systemone_keeps_answers_and_usage() {
    let fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    let output = format_response(
        &fixture.config,
        &decisions,
        &ir_response(Some("upstream-model")),
    )
    .expect("the response formats");
    match output {
        DecisionsOutput::SystemOne(response) => {
            assert_eq!(response.model.as_deref(), Some("upstream-model"));
            assert!(matches!(
                response.answers.get("first"),
                Some(systemone::DecisionAnswer::Noul { noul, .. })
                    if *noul == Number::from_f64(0.9).expect("a finite number")
            ));
            let usage = response.usage.expect("the conversion sets usage");
            assert_eq!(usage.input_tokens.0, Number::from(3_u8));
            assert_eq!(usage.output_tokens.0, Number::from(5_u8));
        }
        DecisionsOutput::OpenAi(_) => {
            panic!("a SystemOne request answers with a SystemOne response")
        }
    }
}

#[test]
fn format_response_systemone_rejects_a_different_answer_count() {
    let fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    let mut response = ir_response(Some("model-1"));
    response.answers.push(ir::DecisionAnswer::Refusal);
    let error =
        format_response(&fixture.config, &decisions, &response).expect_err("the counts differ");
    assert_eq!(answer_count(error), (2, 1));
}

#[test]
fn format_response_openai_uses_the_requested_model() {
    let fixture = Fixture::new();
    let decisions = openai_call("typesafe", None);
    let unnamed = openai_output(&fixture.config, &decisions, None);
    assert_eq!(unnamed.model, "requested/model");
    assert_eq!(
        unnamed.hidden_params.get("model"),
        Some(&Json::Text(String::from("typesafe/model-1")))
    );
    assert_eq!(
        unnamed.hidden_params.get("provider_response_model"),
        Some(&Json::Text(String::from("typesafe/model-1")))
    );
    assert_eq!(
        unnamed.hidden_params.get("custom_llm_provider"),
        Some(&Json::Text(String::from("typesafe")))
    );
    assert!(!unnamed.hidden_params.contains_key("additional_headers"));
    let named = openai_output(&fixture.config, &decisions, Some("provider-model"));
    assert_eq!(named.model, "provider-model");
    let mut config = TestConfig::new(&fixture.recording);
    config.cost = Some(Decimal::new(25, 4));
    let priced = openai_output(&config, &decisions, Some("provider-model"));
    assert_eq!(cost_header(&priced.hidden_params), Some(0.002_5_f64));
}

#[test]
fn format_response_openai_rejects_a_different_answer_count() {
    let fixture = Fixture::new();
    let decisions = openai_call("typesafe", None);
    let mut response = ir_response(Some("model-1"));
    response.answers.push(ir::DecisionAnswer::Refusal);
    let error =
        format_response(&fixture.config, &decisions, &response).expect_err("the counts differ");
    assert_eq!(answer_count(error), (2, 1));
}

#[test]
fn format_response_hidden_params_without_cost() {
    let fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    let output = format_response(&fixture.config, &decisions, &ir_response(Some("model-1")))
        .expect("the response formats");
    let response = match output {
        DecisionsOutput::SystemOne(response) => response,
        DecisionsOutput::OpenAi(_) => {
            panic!("a SystemOne request answers with a SystemOne response")
        }
    };
    assert_eq!(
        response.hidden_params.get("model"),
        Some(&Json::Text(String::from("typesafe/model-1")))
    );
    assert_eq!(
        response.hidden_params.get("provider_response_model"),
        Some(&Json::Text(String::from("typesafe/model-1")))
    );
    assert_eq!(
        response.hidden_params.get("custom_llm_provider"),
        Some(&Json::Text(String::from("typesafe")))
    );
    assert!(!response.hidden_params.contains_key("additional_headers"));
    let serialized = serde_json::to_string(&response).expect("the response serializes");
    assert!(!serialized.contains("hidden_params"));
    assert!(!serialized.contains(RESPONSE_COST_HEADER));
}

#[test]
fn format_response_hidden_params_with_cost() {
    let fixture = Fixture::new();
    let mut config = TestConfig::new(&fixture.recording);
    config.cost = Some(Decimal::new(25, 4));
    let decisions = systemone_call("typesafe", None);
    let output = format_response(&config, &decisions, &ir_response(Some("model-1")))
        .expect("the response formats");
    let response = match output {
        DecisionsOutput::SystemOne(response) => response,
        DecisionsOutput::OpenAi(_) => {
            panic!("a SystemOne request answers with a SystemOne response")
        }
    };
    assert_eq!(cost_header(&response.hidden_params), Some(0.002_5_f64));
    let serialized = serde_json::to_string(&response).expect("the response serializes");
    assert!(!serialized.contains("additional_headers"));
    assert!(!serialized.contains(RESPONSE_COST_HEADER));
}
