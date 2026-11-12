#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use crate::helpers::HelperError;
use rstest::rstest;
use serde_json::{Value, json};

fn input(text: &str) -> Input {
    serde_json::from_str(text).expect("a fixture input parses")
}

fn run(text: &str) -> Value {
    Op::run(&input(text)).expect("the fees compute")
}

fn failure(text: &str) -> Box<HelperError> {
    Op::run(&input(text)).expect_err("the call fails")
}

#[rstest]
#[case::low("low", "0.5")]
#[case::high("high", "1")]
#[case::medium("medium", "0.25")]
#[case::unknown_size_uses_medium("gigantic", "0.25")]
fn a_web_search_size_selects_its_rate(#[case] size: &str, #[case] expected: &str) {
    let text = format!(
        r#"{{"web_search": [{{
            "options": {{"search_context_size": "{size}"}},
            "model_info": {{"search_context_cost_per_query": {{
                "search_context_size_low": 0.5,
                "search_context_size_medium": 0.25,
                "search_context_size_high": 1
            }}}}
        }}]}}"#
    );
    assert_eq!(run(&text)["web_search"], json!([expected]));
}

#[test]
fn a_null_search_rate_is_returned_as_null_and_a_missing_one_is_free() {
    let output = run(r#"{"web_search": [
        {"options": null, "model_info": {"search_context_cost_per_query": {"search_context_size_medium": null}}},
        {"options": {"search_context_size": "high"}, "model_info": {"search_context_cost_per_query": {"search_context_size_low": 0.5}}},
        {"options": {"search_context_size": "low"}, "model_info": null}
    ]}"#);
    assert_eq!(output["web_search"], json!([null, "0", "0"]));
}

#[test]
fn an_empty_file_search_object_still_bills_and_none_does_not() {
    let output = run(r#"{"file_search": [
        {"file_search": {}, "provider": "openai", "model_info": null, "storage_gb": null, "days": null},
        {"file_search": null, "provider": "openai", "model_info": null, "storage_gb": null, "days": null}
    ]}"#);
    assert_eq!(output["file_search"], json!(["0.0025", "0"]));
}

#[test]
fn an_azure_storage_key_blocks_the_per_call_alternative() {
    let output = run(r#"{"file_search": [
        {"file_search": {}, "provider": "azure", "storage_gb": 2, "days": 3,
         "model_info": {"file_search_cost_per_gb_per_day": 0.5, "file_search_cost_per_1k_calls": 7}},
        {"file_search": {}, "provider": "azure", "storage_gb": 2, "days": 0,
         "model_info": {"file_search_cost_per_gb_per_day": 0.5, "file_search_cost_per_1k_calls": 7}},
        {"file_search": {}, "provider": "azure", "storage_gb": 2, "days": 3,
         "model_info": {"file_search_cost_per_1k_calls": 0.5}},
        {"file_search": {}, "provider": "azure", "storage_gb": 2, "days": 3,
         "model_info": {"file_search_cost_per_1k_calls": null}}
    ]}"#);
    assert_eq!(output["file_search"], json!(["3", "0", "0.5", null]));
}

#[test]
fn computer_use_prices_defaults_and_one_sided_custom_rates() {
    let output = run(r#"{"computer_use": [
        {"input_tokens": 1000, "output_tokens": 500, "provider": "azure", "model_info": null},
        {"input_tokens": 1000, "output_tokens": 500, "provider": "azure",
         "model_info": {"computer_use_input_cost_per_1k_tokens": 0, "computer_use_output_cost_per_1k_tokens": 0}},
        {"input_tokens": 1000, "output_tokens": 500, "provider": "azure",
         "model_info": {"computer_use_input_cost_per_1k_tokens": 2}},
        {"input_tokens": 1000, "output_tokens": 500, "provider": "openai", "model_info": null}
    ]}"#);
    assert_eq!(output["computer_use"], json!(["9", "9", "2", "0"]));
}

#[test]
fn vector_store_uses_the_model_rate_then_the_azure_default() {
    let output = run(r#"{"vector_store": [
        {"usage": {"storage_gb": 2, "days": 5}, "provider": "azure", "model_info": null},
        {"usage": {"storage_gb": 2, "days": 5}, "provider": "openai", "model_info": null},
        {"usage": {"storage_gb": 2, "days": 5}, "provider": "openai",
         "model_info": {"vector_store_cost_per_gb_per_day": 0.5}}
    ]}"#);
    assert_eq!(output["vector_store"], json!(["1", "0", "5"]));
}

#[test]
fn code_interpreter_reads_the_container_rate_and_a_free_model_rate() {
    let output = run(r#"{"code_interpreter": [
        {"sessions": 3, "provider": "azure", "model_info": null},
        {"sessions": 3, "provider": "nobody", "model_info": null},
        {"sessions": -2, "provider": "azure", "model_info": {"code_interpreter_cost_per_session": 0.5}},
        {"sessions": 2, "provider": "azure", "model_info": {"code_interpreter_cost_per_session": 0}},
        {"sessions": 0, "provider": "azure", "model_info": null},
        {"sessions": 3, "provider": null, "model_info": null}
    ]}"#);
    assert_eq!(
        output["code_interpreter"],
        json!(["0.09", "0", "-1", "0", "0", "0"])
    );
}

#[test]
fn a_null_rate_meeting_arithmetic_is_a_type_error() {
    let error = failure(
        r#"{"code_interpreter": [
            {"sessions": 2, "provider": "azure", "model_info": {"code_interpreter_cost_per_session": null}}
        ]}"#,
    );
    assert_eq!(error.class, "TypeError");
}

#[test]
fn a_rate_that_is_not_a_number_is_a_value_error() {
    let error = failure(
        r#"{"vector_store": [
            {"usage": {"storage_gb": 1, "days": 1}, "provider": "azure",
             "model_info": {"vector_store_cost_per_gb_per_day": "cheap"}}
        ]}"#,
    );
    assert_eq!(error.class, "ValueError");
}
