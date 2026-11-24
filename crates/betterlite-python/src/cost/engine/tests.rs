#![cfg(test)]

use std::str::FromStr as _;
use std::time::SystemTime;

use betterlite_config::cost_adjustments::CostAdjustmentMap;
use betterlite_core::cost::completion::error::CompletionCostError;
use betterlite_core::cost::prepare::error::PrepareError;
use indexmap::IndexMap;
use pyo3_async_runtimes::tokio::get_runtime;
use rust_decimal::Decimal;

use betterlite_models::decisions::Json;

use crate::cost::billing::{CallRequest, CompletionRequest};
use crate::cost::breakdown::Capture;
use crate::cost::convert::{UsageRequest, parse};
use crate::cost::engine::{Engine, usage_object};
use crate::cost::error::EngineError;
use crate::cost::settings::{Settings, SettingsWire};

const CATALOG: &str = r#"{
    "gpt-test": {
        "litellm_provider": "openai",
        "mode": "chat",
        "input_cost_per_token": 0.001,
        "output_cost_per_token": 0.002
    }
}"#;

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the fixture decimal parses")
}

fn engine() -> Engine {
    let model_cost: IndexMap<String, Json> = parse(CATALOG).expect("the fixture catalog parses");
    let settings = Settings::build(SettingsWire {
        model_cost,
        cost_discount_config: CostAdjustmentMap::default(),
        cost_margin_config: CostAdjustmentMap::default(),
    })
    .expect("the fixture settings build");
    Engine { settings }
}

fn call(text: &str) -> CallRequest {
    parse(text).expect("the per-token request parses")
}

fn completion(text: &str) -> CompletionRequest {
    parse(text).expect("the completion request parses")
}

#[test]
fn cost_per_token_prices_the_prompt_and_the_completion_at_the_model_rates() {
    let request = call(
        r#"{"model": "gpt-test", "prompt_tokens": 10, "completion_tokens": 5,
            "custom_llm_provider": "openai"}"#,
    );
    let (prompt, completion) = engine()
        .cost_per_token(&request)
        .expect("the call is priced");
    assert_eq!(prompt, dec("0.01"));
    assert_eq!(completion, dec("0.01"));
}

#[test]
fn cost_per_token_prices_a_custom_rate_over_the_model_rate() {
    let request = call(
        r#"{"model": "gpt-test", "prompt_tokens": 10, "completion_tokens": 5,
            "custom_llm_provider": "openai",
            "custom_cost_per_token": {"input_cost_per_token": 0.5, "output_cost_per_token": 1.5}}"#,
    );
    let (prompt, completion) = engine()
        .cost_per_token(&request)
        .expect("the call is priced");
    assert_eq!(prompt, dec("5"));
    assert_eq!(completion, dec("7.5"));
}

#[test]
fn cost_per_token_refuses_a_model_it_cannot_price() {
    let request = call(r#"{"model": "no-such-model", "prompt_tokens": 1}"#);
    let error = engine()
        .cost_per_token(&request)
        .expect_err("an unknown model fails");
    assert!(matches!(error, EngineError::Price(_)));
}

#[test]
fn cost_per_token_keeps_the_usage_object_over_the_explicit_counts() {
    let request = call(
        r#"{"model": "gpt-test", "prompt_tokens": 1000, "completion_tokens": 1000,
            "custom_llm_provider": "openai",
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}"#,
    );
    let (prompt, completion) = engine()
        .cost_per_token(&request)
        .expect("the call is priced");
    assert_eq!(prompt, dec("0.01"));
    assert_eq!(completion, dec("0.01"));
}

#[test]
fn cost_charge_prices_a_chat_response_from_its_usage() {
    let request = completion(
        r#"{"model": "gpt-test", "custom_llm_provider": "openai",
            "response": {"kind": "chat", "model": "gpt-test",
                         "usage": {"prompt_tokens": 10, "completion_tokens": 5,
                                   "total_tokens": 15}}}"#,
    );
    let mut clock = SystemTime::now;
    let mut capture = Capture::default();
    let cost = get_runtime()
        .block_on(engine().charge(&request, &mut clock, Some(&mut capture)))
        .expect("the response is priced");
    assert_eq!(cost, dec("0.02"));
    let breakdown = capture.breakdown.expect("the breakdown is recorded");
    assert_eq!(breakdown.input_cost, Some(dec("0.01")));
    assert_eq!(breakdown.output_cost, Some(dec("0.01")));
    assert_eq!(breakdown.total_cost, Some(dec("0.02")));
    assert_eq!(
        capture.rates.map(|rates| rates.input_cost_per_token),
        Some(dec("0.001"))
    );
}

#[test]
fn cost_charge_prices_an_empty_call_at_zero() {
    let request = completion(r#"{"model": "gpt-test", "custom_llm_provider": "openai"}"#);
    let mut clock = SystemTime::now;
    let mut capture = Capture::default();
    let cost = get_runtime()
        .block_on(engine().charge(&request, &mut clock, Some(&mut capture)))
        .expect("the empty call is priced");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn cost_charge_refuses_text_it_would_have_to_tokenize() {
    let request = completion(
        r#"{"model": "gpt-test", "custom_llm_provider": "openai", "prompt": "hello world"}"#,
    );
    let mut clock = SystemTime::now;
    let mut capture = Capture::default();
    let error = get_runtime()
        .block_on(engine().charge(&request, &mut clock, Some(&mut capture)))
        .expect_err("the uncounted text fails");
    assert!(matches!(
        error,
        EngineError::Completion(CompletionCostError::Prepare(PrepareError::Counting(_)))
    ));
}

#[test]
fn cost_charge_refuses_messages_it_would_have_to_tokenize() {
    let request = completion(
        r#"{"model": "gpt-test", "custom_llm_provider": "openai",
            "messages": [{"role": "user", "content": "hello world"}]}"#,
    );
    let mut clock = SystemTime::now;
    let mut capture = Capture::default();
    let error = get_runtime()
        .block_on(engine().charge(&request, &mut clock, Some(&mut capture)))
        .expect_err("the uncounted messages fail");
    assert!(matches!(
        error,
        EngineError::Completion(CompletionCostError::Prepare(PrepareError::Counting(_)))
    ));
}

#[test]
fn cost_charge_prices_the_model_the_response_names_when_the_argument_has_none() {
    let request = completion(
        r#"{"custom_llm_provider": "openai",
            "response": {"kind": "chat", "model": "gpt-test",
                         "usage": {"prompt_tokens": 10, "completion_tokens": 5}}}"#,
    );
    let mut clock = SystemTime::now;
    let mut capture = Capture::default();
    let cost = get_runtime()
        .block_on(engine().charge(&request, &mut clock, Some(&mut capture)))
        .expect("the response is priced");
    assert_eq!(cost, dec("0.02"));
}

#[test]
fn cost_usage_object_reads_the_supplied_usage_and_answers_none_without_one() {
    let request =
        call(r#"{"usage": {"prompt_tokens": 4, "completion_tokens": 2, "total_tokens": 6}}"#);
    let usage = usage_object(&UsageRequest {
        usage: request.usage,
        kind: None,
    })
    .expect("the usage reads");
    assert_eq!(usage.map(|found| found.total_tokens), Some(6));
    let empty = usage_object(&UsageRequest::default()).expect("an absent usage reads as none");
    assert!(empty.is_none());
}

#[test]
fn cost_usage_object_transforms_a_transcription_token_usage() {
    let request: UsageRequest = parse(
        r#"{"kind": "transcription",
            "usage": {"type": "tokens", "input_tokens": 3, "output_tokens": 2, "total_tokens": 5,
                      "input_token_details": {"text_tokens": 3, "audio_tokens": 1}}}"#,
    )
    .expect("the usage request parses");
    let usage = usage_object(&request)
        .expect("the usage reads")
        .expect("a token usage comes back");
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 5);
}

#[test]
fn cost_usage_object_answers_none_for_a_transcription_duration() {
    let request: UsageRequest =
        parse(r#"{"kind": "transcription", "usage": {"type": "duration", "seconds": 2.5}}"#)
            .expect("the usage request parses");
    let usage = usage_object(&request).expect("the usage reads");
    assert!(usage.is_none());
}

#[test]
fn cost_usage_object_transforms_a_decisions_usage() {
    let request: UsageRequest = parse(
        r#"{"kind": "decisions_ir",
            "usage": {"input_tokens": 4, "output_tokens": 2,
                      "cached_tokens": 1, "cache_write_tokens": 1}}"#,
    )
    .expect("the usage request parses");
    let usage = usage_object(&request)
        .expect("the usage reads")
        .expect("a decisions usage comes back");
    assert_eq!(usage.prompt_tokens, 4);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 6);
}

#[test]
fn cost_usage_object_transforms_an_openai_decisions_usage() {
    let request: UsageRequest = parse(
        r#"{"kind": "decisions_openai",
            "usage": {"input_tokens": 4, "output_tokens": 2, "total_tokens": 6,
                      "input_tokens_details": {"cached_tokens": 1, "cache_write_tokens": 1}}}"#,
    )
    .expect("the usage request parses");
    let usage = usage_object(&request)
        .expect("the usage reads")
        .expect("a decisions usage comes back");
    assert_eq!(usage.prompt_tokens, 4);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 6);
}
