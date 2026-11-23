use betterlite_models::decisions::{Json, ir, systemone};
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
    let output =
        format_response(&fixture.config, &decisions, &ir_response(None)).expect("it formats");
    match output {
        DecisionsOutput::OpenAi(response) => assert_eq!(response.model, "requested/model"),
        DecisionsOutput::SystemOne(_) => {
            panic!("an OpenAI request answers with an OpenAI response")
        }
    }
    let named = format_response(
        &fixture.config,
        &decisions,
        &ir_response(Some("provider-model")),
    )
    .expect("the response formats");
    match named {
        DecisionsOutput::OpenAi(response) => assert_eq!(response.model, "provider-model"),
        DecisionsOutput::SystemOne(_) => {
            panic!("an OpenAI request answers with an OpenAI response")
        }
    }
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
    let headers = response
        .hidden_params
        .get("additional_headers")
        .expect("the cost header is set");
    match headers {
        Json::Object(entries) => {
            assert_eq!(entries.len(), 1);
            match entries.get(RESPONSE_COST_HEADER) {
                Some(Json::Number(cost)) => assert_eq!(cost.as_f64(), Some(0.002_5_f64)),
                other => panic!("the cost header holds a number, got {other:?}"),
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("additional_headers is a map")
        }
    }
    let serialized = serde_json::to_string(&response).expect("the response serializes");
    assert!(!serialized.contains("additional_headers"));
    assert!(!serialized.contains(RESPONSE_COST_HEADER));
}
