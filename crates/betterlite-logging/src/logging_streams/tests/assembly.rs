use std::str::FromStr as _;
use std::sync::Mutex;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::support::{FakeAssembly, json, state};
use crate::fields::usage::Usage;
use crate::logging_state::LoggingState;
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

fn recovering_usage() -> Usage {
    Usage {
        prompt_tokens: 4,
        completion_tokens: 3,
        total_tokens: 7,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    }
}

fn cost_json(value: &str) -> Json {
    Json::Number(Number::from_str(value).expect("a number"))
}

fn combined_usage(state: &LoggingState) -> Json {
    state
        .model_call_details
        .get("combined_usage_object")
        .cloned()
        .expect("the partial handoff records the combined usage object")
}

fn field(recorded: &Json, name: &str) -> Json {
    let Json::Object(fields) = recorded else {
        panic!("the combined usage is an object");
    };
    fields
        .get(name)
        .cloned()
        .expect("the combined usage carries the field")
}

fn cached_tokens(recorded: &Json) -> Json {
    let Json::Object(details) = field(recorded, "prompt_tokens_details") else {
        panic!("the combined usage carries prompt tokens details");
    };
    details
        .get("cached_tokens")
        .cloned()
        .expect("the combined usage carries cached tokens")
}

fn partial_response(details: &str) -> Json {
    json(&format!(
        r#"{{"model":"gpt-4o","usage":{{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"prompt_tokens_details":{details}}}}}"#
    ))
}

fn test_hands_over_a_complete_response() {
    let assembly = FakeAssembly::complete(completed());
    let build = successful_stream_handoff(&assembly, None);
    assert!(matches!(build, StreamBuild::Complete(response) if response == completed()));
}

fn test_hands_over_nothing_when_the_assembly_is_empty() {
    let assembly = FakeAssembly::empty();
    let build = successful_stream_handoff(&assembly, None);
    assert!(matches!(build, StreamBuild::Empty));
}

fn test_hands_over_usage_when_the_builder_fails() {
    let mut assembly = FakeAssembly::failing();
    assembly.usage = recovering_usage();
    let build = successful_stream_handoff(&assembly, None);
    assert!(matches!(build, StreamBuild::UsageOnly(usage) if usage == recovering_usage()));
}

fn test_passes_the_prompt_counter_through() {
    let assembly = FakeAssembly::complete(completed());
    let held: &PromptCounter<'_> = &counter;
    let _build = successful_stream_handoff(&assembly, Some(held));
    assert_eq!(assembly.counter_value(), Some(Some(7)));
}

fn test_passes_a_missing_prompt_counter_through() {
    let assembly = FakeAssembly::complete(completed());
    let _build = successful_stream_handoff(&assembly, None);
    assert_eq!(assembly.counter_value(), Some(None));
}

fn test_records_partial_usage_and_cost() {
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
    assert_eq!(
        state.model_call_details.get("response_cost"),
        Some(&cost_json("3.0"))
    );
    let recorded = combined_usage(&state);
    assert_eq!(field(&recorded, "prompt_tokens"), json("10"));
    assert_eq!(field(&recorded, "completion_tokens"), json("5"));
    assert_eq!(field(&recorded, "total_tokens"), json("15"));
    assert_eq!(
        seen.into_inner().unwrap(),
        Some(Json::Text("gpt-4o".to_owned()))
    );
}

fn test_records_a_zero_cost_when_the_calculator_gives_none() {
    let mut state = state(Some(true));
    let assembly = FakeAssembly::complete(completed());
    let cost = partial_stream_handoff(&assembly, None, &mut state, |_response| None);
    assert_eq!(cost, Some(Decimal::ZERO));
    assert_eq!(
        state.model_call_details.get("response_cost"),
        Some(&cost_json("0.0"))
    );
    let recorded = combined_usage(&state);
    assert_eq!(field(&recorded, "prompt_tokens"), json("10"));
    assert_eq!(field(&recorded, "completion_tokens"), json("5"));
    assert_eq!(field(&recorded, "total_tokens"), json("15"));
}

fn test_backfills_cache_usage_from_prompt_details() {
    let mut state = state(Some(true));
    let assembly = FakeAssembly::complete(partial_response(
        r#"{"cached_tokens":12,"cache_write_tokens":4}"#,
    ));
    let cost = partial_stream_handoff(&assembly, None, &mut state, |_response| Some(Decimal::ONE));
    assert_eq!(cost, Some(Decimal::ONE));
    let recorded = combined_usage(&state);
    assert_eq!(field(&recorded, "cache_read_input_tokens"), json("12"));
    assert_eq!(field(&recorded, "cache_creation_input_tokens"), json("4"));
    assert_eq!(cached_tokens(&recorded), json("12"));
}

fn test_defaults_missing_cache_usage_to_zero() {
    let mut state = state(Some(true));
    let assembly = FakeAssembly::complete(json(
        r#"{"model":"gpt-4o","usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
    ));
    let cost = partial_stream_handoff(&assembly, None, &mut state, |_response| Some(Decimal::ONE));
    assert_eq!(cost, Some(Decimal::ONE));
    let recorded = combined_usage(&state);
    assert_eq!(field(&recorded, "cache_read_input_tokens"), json("0"));
    assert_eq!(field(&recorded, "cache_creation_input_tokens"), json("0"));
    assert_eq!(cached_tokens(&recorded), json("0"));
}

fn test_keeps_cache_usage_the_provider_already_set() {
    let mut state = state(Some(true));
    let assembly = FakeAssembly::complete(json(
        r#"{"model":"gpt-4o","usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,"cache_read_input_tokens":9,"prompt_tokens_details":{"cached_tokens":12}}}"#,
    ));
    let cost = partial_stream_handoff(&assembly, None, &mut state, |_response| Some(Decimal::ONE));
    assert_eq!(cost, Some(Decimal::ONE));
    let recorded = combined_usage(&state);
    assert_eq!(field(&recorded, "cache_read_input_tokens"), json("9"));
    assert_eq!(cached_tokens(&recorded), json("9"));
}

fn test_backfills_cache_creation_from_creation_details() {
    let mut state = state(Some(true));
    let assembly = FakeAssembly::complete(partial_response(r#"{"cache_creation_tokens":6}"#));
    let cost = partial_stream_handoff(&assembly, None, &mut state, |_response| Some(Decimal::ONE));
    assert_eq!(cost, Some(Decimal::ONE));
    let recorded = combined_usage(&state);
    assert_eq!(field(&recorded, "cache_read_input_tokens"), json("0"));
    assert_eq!(field(&recorded, "cache_creation_input_tokens"), json("6"));
}

fn test_backfills_cache_creation_from_nested_details() {
    let mut state = state(Some(true));
    let assembly = FakeAssembly::complete(partial_response(r#"{"cache_creation_input_tokens":5}"#));
    let cost = partial_stream_handoff(&assembly, None, &mut state, |_response| Some(Decimal::ONE));
    assert_eq!(cost, Some(Decimal::ONE));
    let recorded = combined_usage(&state);
    assert_eq!(field(&recorded, "cache_creation_input_tokens"), json("5"));
}

fn test_keeps_the_assembled_model_when_the_call_has_no_model() {
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

fn test_skips_partial_usage_without_a_usage_object() {
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

fn test_skips_partial_usage_that_is_not_an_object() {
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

fn test_skips_partial_usage_when_the_builder_fails() {
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

fn test_skips_partial_usage_when_the_assembly_is_empty() {
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
#[case::handoff_hands_over_a_complete_response(test_hands_over_a_complete_response)]
#[case::handoff_hands_over_nothing_when_empty(test_hands_over_nothing_when_the_assembly_is_empty)]
#[case::handoff_hands_over_usage_when_the_builder_fails(
    test_hands_over_usage_when_the_builder_fails
)]
#[case::handoff_passes_the_prompt_counter_through(test_passes_the_prompt_counter_through)]
#[case::handoff_passes_a_missing_counter_through(test_passes_a_missing_prompt_counter_through)]
#[case::partial_handoff_records_usage_and_cost(test_records_partial_usage_and_cost)]
#[case::partial_handoff_records_a_zero_cost(
    test_records_a_zero_cost_when_the_calculator_gives_none
)]
#[case::partial_handoff_backfills_cache_usage(test_backfills_cache_usage_from_prompt_details)]
#[case::partial_handoff_defaults_missing_cache_usage(test_defaults_missing_cache_usage_to_zero)]
#[case::partial_handoff_keeps_provider_cache_usage(test_keeps_cache_usage_the_provider_already_set)]
#[case::partial_handoff_backfills_cache_creation(
    test_backfills_cache_creation_from_creation_details
)]
#[case::partial_handoff_backfills_nested_cache_creation(
    test_backfills_cache_creation_from_nested_details
)]
#[case::partial_handoff_keeps_the_routed_model(
    test_keeps_the_assembled_model_when_the_call_has_no_model
)]
#[case::partial_handoff_skips_a_response_without_usage(
    test_skips_partial_usage_without_a_usage_object
)]
#[case::partial_handoff_skips_a_non_object_usage(test_skips_partial_usage_that_is_not_an_object)]
#[case::partial_handoff_skips_a_builder_failure(test_skips_partial_usage_when_the_builder_fails)]
#[case::partial_handoff_skips_an_empty_assembly(
    test_skips_partial_usage_when_the_assembly_is_empty
)]
fn logging_streams_behavior_tests_behavior(#[case] behavior: fn()) {
    behavior();
}
