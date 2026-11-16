use std::sync::Mutex;

use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{FakeAssembly, json, state};
use crate::fields::usage::Usage;
use crate::logging_streams::assembly::{
    PromptCountError, PromptCounter, StreamBuild, partial_stream_handoff, successful_stream_handoff,
};

fn counter() -> Result<i64, PromptCountError> {
    Ok(7)
}

fn completed() -> Json {
    json(
        r#"{"id":"chatcmpl-1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
    )
}

fn hands_over_a_complete_response() {
    let assembly = FakeAssembly::complete(completed());
    let build = successful_stream_handoff(&assembly, None);
    assert!(matches!(build, StreamBuild::Complete(response) if response == completed()));
}

fn hands_over_nothing_when_the_assembly_is_empty() {
    let assembly = FakeAssembly::empty();
    let build = successful_stream_handoff(&assembly, None);
    assert!(matches!(build, StreamBuild::Empty));
}

fn hands_over_usage_when_the_builder_fails() {
    let mut assembly = FakeAssembly::failing();
    assembly.usage = Usage::default();
    let build = successful_stream_handoff(&assembly, None);
    assert!(matches!(build, StreamBuild::UsageOnly(usage) if usage == Usage::default()));
}

fn passes_the_prompt_counter_through() {
    let assembly = FakeAssembly::complete(completed());
    let held: &PromptCounter<'_> = &counter;
    let _build = successful_stream_handoff(&assembly, Some(held));
    assert_eq!(assembly.counter_value(), Some(Some(7)));
}

fn passes_a_missing_prompt_counter_through() {
    let assembly = FakeAssembly::complete(completed());
    let _build = successful_stream_handoff(&assembly, None);
    assert_eq!(assembly.counter_value(), Some(None));
}

fn records_partial_usage_and_cost() {
    let mut state = state(Some(true));
    let assembly = FakeAssembly::complete(completed());
    let seen = Mutex::new(None);
    let cost = partial_stream_handoff(&assembly, None, &mut state, |response| {
        if let Json::Object(fields) = response {
            *seen.lock().unwrap() = fields.get("model").cloned();
        }
        Some(Decimal::from(3))
    });
    assert_eq!(cost, Some(Decimal::from(3)));
    let recorded = state
        .model_call_details
        .get("combined_usage_object")
        .cloned();
    assert!(matches!(recorded, Some(Json::Object(_))));
    assert!(state.model_call_details.contains_key("response_cost"));
    assert_eq!(
        seen.into_inner().unwrap(),
        Some(Json::Text("gpt-4o".to_owned()))
    );
}

fn records_a_zero_cost_when_the_calculator_gives_none() {
    let mut state = state(Some(true));
    let assembly = FakeAssembly::complete(completed());
    let cost = partial_stream_handoff(&assembly, None, &mut state, |_response| None);
    assert_eq!(cost, Some(Decimal::ZERO));
    assert!(state.model_call_details.contains_key("response_cost"));
}

fn keeps_the_assembled_model_when_the_call_has_no_model() {
    let mut state = state(Some(true));
    state.model = String::new();
    let response = json(
        r#"{"model":"routed/model","usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
    );
    let assembly = FakeAssembly::complete(response);
    let seen = Mutex::new(None);
    let _cost = partial_stream_handoff(&assembly, None, &mut state, |response| {
        if let Json::Object(fields) = response {
            *seen.lock().unwrap() = fields.get("model").cloned();
        }
        None
    });
    assert_eq!(
        seen.into_inner().unwrap(),
        Some(Json::Text("routed/model".to_owned()))
    );
}

fn skips_partial_usage_without_a_usage_object() {
    let mut state = state(Some(true));
    let assembly = FakeAssembly::complete(json(r#"{"id":"chatcmpl-1"}"#));
    let cost = partial_stream_handoff(&assembly, None, &mut state, |_response| None);
    assert_eq!(cost, None);
    assert!(
        !state
            .model_call_details
            .contains_key("combined_usage_object")
    );
}

fn skips_partial_usage_that_is_not_an_object() {
    let mut state = state(Some(true));
    let assembly = FakeAssembly::complete(json(r#"{"usage":true}"#));
    let cost = partial_stream_handoff(&assembly, None, &mut state, |_response| None);
    assert_eq!(cost, None);
    assert!(
        !state
            .model_call_details
            .contains_key("combined_usage_object")
    );
}

fn skips_partial_usage_when_the_builder_fails() {
    let mut state = state(Some(true));
    let assembly = FakeAssembly::failing();
    let cost = partial_stream_handoff(&assembly, None, &mut state, |_response| None);
    assert_eq!(cost, None);
    assert!(
        !state
            .model_call_details
            .contains_key("combined_usage_object")
    );
}

fn skips_partial_usage_when_the_assembly_is_empty() {
    let mut state = state(Some(true));
    let assembly = FakeAssembly::empty();
    let cost = partial_stream_handoff(&assembly, None, &mut state, |_response| None);
    assert_eq!(cost, None);
    assert!(
        !state
            .model_call_details
            .contains_key("combined_usage_object")
    );
}

#[rstest]
#[case::handoff_hands_over_a_complete_response(hands_over_a_complete_response)]
#[case::handoff_hands_over_nothing_when_empty(hands_over_nothing_when_the_assembly_is_empty)]
#[case::handoff_hands_over_usage_when_the_builder_fails(hands_over_usage_when_the_builder_fails)]
#[case::handoff_passes_the_prompt_counter_through(passes_the_prompt_counter_through)]
#[case::handoff_passes_a_missing_counter_through(passes_a_missing_prompt_counter_through)]
#[case::partial_handoff_records_usage_and_cost(records_partial_usage_and_cost)]
#[case::partial_handoff_records_a_zero_cost(records_a_zero_cost_when_the_calculator_gives_none)]
#[case::partial_handoff_keeps_the_routed_model(
    keeps_the_assembled_model_when_the_call_has_no_model
)]
#[case::partial_handoff_skips_a_response_without_usage(skips_partial_usage_without_a_usage_object)]
#[case::partial_handoff_skips_a_non_object_usage(skips_partial_usage_that_is_not_an_object)]
#[case::partial_handoff_skips_a_builder_failure(skips_partial_usage_when_the_builder_fails)]
#[case::partial_handoff_skips_an_empty_assembly(skips_partial_usage_when_the_assembly_is_empty)]
fn logging_streams_behavior_tests_behavior(#[case] behavior: fn()) {
    behavior();
}
