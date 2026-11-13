#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;

fn run_case(file: &str) -> Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/groq-chat")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn credentials_prefer_explicit_then_environment_then_the_default() {
    let credentials = &run_case("credentials-and-base.yaml")["credentials"];
    assert_eq!(credentials[0]["api_base"], "https://explicit.test");
    assert_eq!(credentials[0]["api_key"], "explicit-key");
    assert_eq!(credentials[1]["api_base"], "https://environment.test");
    assert_eq!(credentials[1]["api_key"], "environment-key");
    assert_eq!(credentials[2]["api_base"], "https://api.groq.com/openai/v1");
    assert_eq!(credentials[2]["api_key"], Value::Null);
    assert_eq!(credentials[3], credentials[2]);
}

#[test]
fn a_capable_model_keeps_web_search_and_appends_reasoning_effort() {
    let params = &run_case("supported-params-capabilities.yaml")["supported"][0]["params"];
    let names: Vec<&str> = params
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(names.contains(&"web_search_options"));
    assert!(!names.contains(&"max_retries"));
    assert_eq!(names.last(), Some(&"reasoning_effort"));
}

#[test]
fn a_compound_model_keeps_web_search_options_without_reasoning_effort() {
    let entries = &run_case("supported-params-compound-model.yaml")["supported"];
    assert!(
        entries[0]["params"]
            .as_array()
            .unwrap()
            .contains(&json!("web_search_options"))
    );
    assert!(
        !entries[0]["params"]
            .as_array()
            .unwrap()
            .contains(&json!("reasoning_effort"))
    );
    assert!(
        entries[2]["params"]
            .as_array()
            .unwrap()
            .contains(&json!("web_search_options"))
    );
    assert!(
        !entries[2]["params"]
            .as_array()
            .unwrap()
            .contains(&json!("reasoning_effort"))
    );
}

#[test]
fn a_failed_reasoning_lookup_leaves_the_list_without_reasoning_effort() {
    let params =
        &run_case("supported-params-reasoning-lookup-failure.yaml")["supported"][0]["params"];
    assert!(
        !params
            .as_array()
            .unwrap()
            .contains(&json!("reasoning_effort"))
    );
}

#[test]
fn assistant_null_members_are_dropped_and_other_roles_are_kept() {
    let messages = &run_case("assistant-null-members.yaml")["messages"][0]["messages"];
    assert!(messages[0].get("content").is_none());
    assert!(messages[0].get("custom_null").is_none());
    assert!(messages[0].get("function_call").is_none());
    assert_eq!(messages[0]["tool_calls"][0]["id"], "call_1");
    assert_eq!(messages[1]["custom_null"], Value::Null);
}

#[test]
fn an_unsupported_schema_becomes_a_json_tool_call() {
    let mapped = &run_case("response-format-tool-workaround.yaml")["options"][0];
    assert_eq!(
        mapped["optional_params"]["tools"][0]["function"]["name"],
        "json_tool_call"
    );
    assert_eq!(mapped["optional_params"]["json_mode"], true);
    assert_eq!(mapped["optional_params"]["fake_stream"], true);
    assert!(mapped["non_default"].get("response_format").is_none());
}

#[test]
fn a_native_schema_model_keeps_its_response_format() {
    let mapped = &run_case("response-format-native-schema.yaml")["options"][0];
    assert_eq!(
        mapped["optional_params"]["response_format"]["type"],
        "json_schema"
    );
    assert!(mapped["optional_params"].get("tools").is_none());
}

#[test]
fn user_tools_with_a_schema_fail_with_400() {
    let entry = &run_case("response-format-with-tools-error.yaml")["options"][0];
    assert_eq!(entry["error"]["status"], 400);
}

#[test]
fn web_search_options_add_one_tool_without_duplicating_it() {
    let entries = &run_case("web-search-options-tool.yaml")["options"];
    assert_eq!(
        entries[0]["optional_params"]["tools"],
        json!([{"type": "browser_search"}])
    );
    assert_eq!(
        entries[1]["optional_params"]["tools"],
        json!([{"type": "browser_search"}])
    );
    assert!(entries[2]["optional_params"].get("tools").is_none());
    assert!(entries[3]["optional_params"].get("tools").is_none());
}

#[test]
fn max_completion_tokens_is_sent_as_max_tokens() {
    let mapped = &run_case("max-completion-tokens-swap.yaml")["options"][0];
    assert_eq!(mapped["optional_params"]["max_tokens"], 128);
    assert!(
        mapped["optional_params"]
            .get("max_completion_tokens")
            .is_none()
    );
}

#[test]
fn service_tier_maps_to_auto_for_null_and_unknown_values() {
    let replies = &run_case("service-tier-mapping.yaml")["replies"];
    assert_eq!(replies[0]["service_tier"], "auto");
    assert_eq!(replies[1]["service_tier"], "flex");
    assert_eq!(replies[2]["service_tier"], "auto");
    assert_eq!(replies[3]["service_tier"], "default");
}

#[test]
fn browser_actions_are_counted_and_a_missing_usage_is_not_billed() {
    let replies = &run_case("executed-tools-counts.yaml")["replies"];
    assert_eq!(replies[0]["web_search_requests"], 2);
    assert_eq!(replies[0]["browser_open_requests"], 1);
    assert_eq!(replies[1]["web_search_requests"], 0);
    assert_eq!(replies[1]["browser_open_requests"], 1);
    assert_eq!(replies[2]["web_search_requests"], Value::Null);
    assert_eq!(replies[2]["browser_open_requests"], Value::Null);
}

#[test]
fn a_malformed_executed_tools_list_bills_nothing() {
    let replies = &run_case("executed-tools-bad-shape.yaml")["replies"];
    assert_eq!(replies[0]["web_search_requests"], Value::Null);
    assert_eq!(replies[1]["web_search_requests"], Value::Null);
    assert_eq!(replies[2]["web_search_requests"], Value::Null);
}

#[test]
fn stream_chunks_carry_reasoning_content_and_error_chunks_fail() {
    let chunks = &run_case("stream-chunk-error-and-reasoning.yaml")["chunks"];
    assert_eq!(chunks[0]["choices"][0]["reasoning_content"], "thinking");
    assert_eq!(chunks[0]["choices"][0]["content"], "answer");
    assert_eq!(chunks[1]["object"], "chat.completion.chunk");
    assert_eq!(chunks[2]["error"]["status"], Value::Null);
    assert_eq!(chunks[3]["error"]["status"], Value::Null);
}

#[test]
fn transcription_params_copy_or_drop_by_response_format() {
    let entries = &run_case("stt-options.yaml")["stt"];
    assert_eq!(
        entries[0]["optional_params"]["response_format"],
        "verbose_json"
    );
    assert_eq!(entries[0]["optional_params"]["prompt"], "keep");
    assert_eq!(entries[1]["optional_params"]["response_format"], "text");
    assert_eq!(entries[2]["optional_params"]["prompt"], "new");
}

#[test]
fn an_unsupported_transcription_format_is_dropped_or_fails() {
    let entries = &run_case("stt-unsupported-format.yaml")["stt"];
    assert_eq!(entries[0]["error"]["status"], 400);
    assert_eq!(entries[1]["optional_params"]["temperature"], 1);
    assert!(
        entries[1]["optional_params"]
            .get("response_format")
            .is_none()
    );
    assert_eq!(entries[2]["optional_params"]["temperature"], 1);
}

#[test]
fn search_and_browser_open_costs_are_exact_decimals() {
    let costs = &run_case("web-search-cost.yaml")["costs"];
    assert_eq!(costs[0]["cost"], "0.013000000000");
    assert_eq!(costs[1]["cost"], "0.004000000000");
    assert_eq!(costs[2]["cost"], "0.000000000000");
    assert_eq!(costs[3]["cost"], "0.015000000000");
    assert_eq!(costs[4]["cost"], "0.010000000000");
}
