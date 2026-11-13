#![cfg(test)]

mod corpus;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::Object;
use rstest::rstest;
use serde::Deserialize;
use serde_json::Value;

use super::error::ResponseInputError;
use super::{ResponsesInput, transform_responses_request};

#[derive(Deserialize)]
struct Probe {
    model: String,
    input: ResponsesInput,
    #[serde(default)]
    params: Object,
    #[serde(default)]
    provider: Option<ProviderId>,
    #[serde(default)]
    model_info: Option<Object>,
}

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).expect("the fixture parses")
}

fn probe(value: Value) -> Probe {
    serde_json::from_value(value).expect("the probe decodes")
}

fn run(case: &Probe) -> Result<Object, ResponseInputError> {
    transform_responses_request(
        &case.model,
        &case.input,
        &case.params,
        case.provider.unwrap_or(ProviderId::Openai),
        case.model_info.as_ref(),
    )
}

fn expect_body(value: Value) -> Value {
    let body = run(&probe(value)).expect("the request is prepared");
    serde_json::to_value(body).expect("the body serializes")
}

fn expect_tool_error(value: Value) -> ResponseInputError {
    run(&probe(value)).expect_err("the tool schema is rejected")
}

fn assert_prepared(probe: &str, expected: &str) {
    assert_eq!(expect_body(fixture(probe)), fixture(expected));
}

fn schema_body(model: &str, parameters: &str) -> Value {
    fixture(&format!(
        r#"{{"model": "{model}", "input": "hi", "tools": [{{"type": "function", "name": "f", "parameters": {parameters}}}]}}"#
    ))
}

fn combiner_request(model: &str) -> Value {
    fixture(&format!(
        r#"{{"model": "{model}", "input": "hi", "params": {{"tools": [{{"type": "function", "name": "f", "parameters": {COMBINER}}}]}}}}"#
    ))
}

fn provider_combiner_request(model: &str, provider: &str) -> Value {
    fixture(&format!(
        r#"{{"model": "{model}", "provider": "{provider}", "input": "hi", "params": {{"tools": [{{"type": "function", "name": "f", "parameters": {COMBINER}}}]}}}}"#
    ))
}

fn gate_request(model: &str, model_info: &str) -> Value {
    fixture(&format!(
        r#"{{"model": "{model}", "input": "hi", "params": {{"tools": [{{"type": "function", "name": "f", "parameters": {COMBINER}}}]}}, "model_info": {model_info}}}"#
    ))
}

const COMBINER: &str = r#"{"type": "object", "anyOf": [{"type": "string"}], "properties": {"bad": {"pattern": "("}, "good": {"pattern": "^a+$"}}}"#;
const FLATTENED: &str =
    r#"{"type": "object", "properties": {"bad": {}, "good": {"pattern": "^a+$"}}}"#;
const KEPT_WITH_REGEX_DROPPED: &str = r#"{"type": "object", "anyOf": [{"type": "string"}], "properties": {"bad": {}, "good": {"pattern": "^a+$"}}}"#;
const SUMMARY_ENTRY: &str = r#"{"type": "summary_text", "text": "x"}"#;

fn assert_flattened(model: &str) {
    assert_eq!(
        expect_body(combiner_request(model)),
        schema_body(model, FLATTENED)
    );
}

fn assert_combinators_kept(model: &str) {
    assert_eq!(
        expect_body(combiner_request(model)),
        schema_body(model, KEPT_WITH_REGEX_DROPPED)
    );
}

fn assert_gate(model: &str, model_info: &str, parameters: &str) {
    assert_eq!(
        expect_body(gate_request(model, model_info)),
        schema_body(model, parameters)
    );
}

fn string_schema_object() {
    assert_prepared(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"tools": [
            {"type": "function", "name": "f", "parameters": "{\"type\":\"object\"}"},
            {"type": "function", "name": "g", "parameters": {"type": "object", "title": "kept"}},
            {"type": "function", "name": "h", "parameters": null},
            {"type": "function", "name": "i"},
            "raw-tool"
        ]}}"#,
        r#"{"model": "gpt-4o", "input": "hi", "tools": [
            {"type": "function", "name": "f", "parameters": {"type": "object"}},
            {"type": "function", "name": "g", "parameters": {"type": "object", "title": "kept"}},
            {"type": "function", "name": "h", "parameters": null},
            {"type": "function", "name": "i"},
            "raw-tool"
        ]}"#,
    );
    let spaced = expect_body(fixture(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"tools": [{"parameters": " { \"type\" : \"object\" , \"n\" : 1 } "}]}}"#,
    ));
    assert_eq!(
        spaced["tools"][0]["parameters"],
        fixture(r#"{"type": "object", "n": 1}"#)
    );
}

fn assert_non_object_string_schema() {
    let error = expect_tool_error(fixture(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"tools": [{"parameters": "[1]"}]}}"#,
    ));
    assert_eq!(error.http_status(), Some(400));
    assert!(error.to_string().contains("tools[0].parameters"));
    assert!(error.to_string().contains("str"));
    let ResponseInputError::InvalidToolParameters {
        model,
        provider,
        index,
        found,
    } = error
    else {
        panic!("expected the tools parameters error");
    };
    assert_eq!(model, "gpt-4o");
    assert_eq!(provider, ProviderId::Openai);
    assert_eq!(index, 0);
    assert_eq!(found, "str");
}

fn assert_malformed_string_schema() {
    let error = expect_tool_error(fixture(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"tools": [{"name": "ok"}, {"name": "bad", "parameters": "{bad"}]}}"#,
    ));
    assert!(error.to_string().contains("tools[1].parameters"));
    assert!(error.to_string().contains("str"));
}

fn assert_parameters_error(parameters: &str, expected: &str) {
    let error = expect_tool_error(fixture(&format!(
        r#"{{"model": "gpt-4o", "input": "hi", "params": {{"tools": [{{"parameters": {parameters}}}]}}}}"#
    )));
    assert_eq!(error.http_status(), Some(400));
    assert!(error.to_string().contains(expected), "{error}");
}

fn invalid_string_schema() {
    assert_non_object_string_schema();
    assert_malformed_string_schema();
    assert_parameters_error("5", "int");
    assert_parameters_error("1.5", "float");
    assert_parameters_error("true", "bool");
    assert_parameters_error("[]", "list");
    assert_parameters_error(r#""null""#, "str");
}

fn recursive_tool_namespace() {
    assert_prepared(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"tools": [
            {
                "type": "namespace",
                "name": "ns",
                "parameters": {"type": "object", "anyOf": [{"type": "string"}]},
                "tools": [
                    {"type": "function", "name": "inner", "parameters": "{\"type\":\"object\"}"},
                    "raw-inner",
                    {"type": "function", "name": "deep", "parameters": {"anyOf": [{"type": "number"}], "properties": {"p": {"pattern": "("}}}}
                ],
                "unrelated": {"keep": true}
            },
            {"type": "function", "name": "plain", "parameters": {"type": "object"}}
        ]}}"#,
        r#"{"model": "gpt-4o", "input": "hi", "tools": [
            {
                "type": "namespace",
                "name": "ns",
                "parameters": {"type": "object", "properties": {}},
                "tools": [
                    {"type": "function", "name": "inner", "parameters": "{\"type\":\"object\"}"},
                    "raw-inner",
                    {"type": "function", "name": "deep", "parameters": {"properties": {"p": {}}, "type": "object"}}
                ],
                "unrelated": {"keep": true}
            },
            {"type": "function", "name": "plain", "parameters": {"type": "object"}}
        ]}"#,
    );
}

fn assert_foreign_ids_openai() {
    assert_prepared(
        r#"{"model": "gpt-4o", "input": [
            {"type": "function_call", "id": "call_1", "call_id": "c1", "name": "f"},
            {"type": "function_call", "id": "fc_1", "call_id": "c2"},
            {"type": "function_call", "id": "fc", "call_id": "c3"},
            {"type": "custom_tool_call", "id": "bad", "call_id": "c4"},
            {"type": "custom_tool_call", "id": "ctc_1"},
            {"type": "custom_tool_call", "id": "ctcx"},
            {"type": "function_call", "id": 5},
            {"type": "message", "id": "msg_1", "role": "user"},
            "plain-string-item"
        ], "params": {}}"#,
        r#"{"model": "gpt-4o", "input": [
            {"type": "function_call", "call_id": "c1", "name": "f"},
            {"type": "function_call", "id": "fc_1", "call_id": "c2"},
            {"type": "function_call", "id": "fc", "call_id": "c3"},
            {"type": "custom_tool_call", "call_id": "c4"},
            {"type": "custom_tool_call", "id": "ctc_1"},
            {"type": "custom_tool_call", "id": "ctcx"},
            {"type": "function_call", "id": 5},
            {"type": "message", "id": "msg_1", "role": "user"},
            "plain-string-item"
        ]}"#,
    );
}

fn assert_foreign_ids_azure() {
    assert_prepared(
        r#"{"model": "azure/dep", "provider": "azure", "input": [
            {"type": "function_call", "id": "call_9", "call_id": "c9"},
            {"type": "function_call", "id": "fcz"}
        ], "params": {}}"#,
        r#"{"model": "azure/dep", "input": [
            {"type": "function_call", "call_id": "c9"},
            {"type": "function_call", "id": "fcz"}
        ]}"#,
    );
}

fn foreign_tool_item_id() {
    assert_foreign_ids_openai();
    assert_foreign_ids_azure();
    assert_prepared(
        r#"{"model": "gpt-4o", "provider": "hosted_vllm", "input": [{"type": "function_call", "id": "call_7"}], "params": {}}"#,
        r#"{"model": "gpt-4o", "input": [{"type": "function_call", "id": "call_7"}]}"#,
    );
    assert_prepared(
        r#"{"model": "gpt-4o", "input": "just a string", "params": {}}"#,
        r#"{"model": "gpt-4o", "input": "just a string"}"#,
    );
}

fn cache_control_in_input_and_tools() {
    assert_prepared(
        r#"{"model": "gpt-4o", "input": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": "hi", "cache_control": {"type": "ephemeral"}},
                    "raw-part",
                    {"type": "input_image", "image_url": "u", "nested": {"cache_control": 1, "keep": 2}}
                ],
                "cache_control": {"t": 1}
            },
            "raw-item",
            {"type": "reasoning", "id": "r", "summary": [], "status": null, "cache_control": 7}
        ], "params": {"tools": [
            {"type": "function", "name": "f", "cache_control": {"x": 1}, "parameters": {"type": "object", "cache_control": {"deep": 1}}},
            {"parameters": "{\"cache_control\":{\"type\":\"ephemeral\"},\"type\":\"object\"}"},
            "raw-tool"
        ]}}"#,
        r#"{"model": "gpt-4o", "input": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": "hi"},
                    "raw-part",
                    {"type": "input_image", "image_url": "u", "nested": {"keep": 2}}
                ]
            },
            "raw-item",
            {"id": "r", "summary": [], "type": "reasoning"}
        ], "tools": [
            {"type": "function", "name": "f", "parameters": {"type": "object"}},
            {"parameters": {"cache_control": {"type": "ephemeral"}, "type": "object"}},
            "raw-tool"
        ]}"#,
    );
    assert_cache_control_runs_before_decoding();
}

fn assert_cache_control_runs_before_decoding() {
    let decoded_and_sanitized = expect_body(fixture(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"tools": [{"parameters": "{\"anyOf\":[{\"type\":\"string\"}],\"type\":\"object\"}"}]}}"#,
    ));
    assert_eq!(
        decoded_and_sanitized["tools"][0]["parameters"],
        fixture(r#"{"type": "object"}"#)
    );
}

fn assert_reasoning_validated() {
    assert_prepared(
        r#"{"model": "gpt-4o", "input": [
            {
                "type": "reasoning",
                "id": "r1",
                "summary": [{"type": "summary_text", "text": "s", "drop": null, "keep": {"n": null}}],
                "content": [{"type": "reasoning_text", "text": "c", "drop": null}],
                "encrypted_content": "enc",
                "status": "completed",
                "extra": null,
                "extra_object": {"n": null}
            },
            {"type": "reasoning", "id": "r2", "reasoning_content": [{"type": "summary_text", "text": "from-content"}]},
            {"type": "message", "role": "user", "status": null}
        ], "params": {}}"#,
        r#"{"model": "gpt-4o", "input": [
            {
                "id": "r1",
                "summary": [{"text": "s", "type": "summary_text", "keep": {"n": null}}],
                "type": "reasoning",
                "content": [{"text": "c", "type": "reasoning_text"}],
                "encrypted_content": "enc",
                "status": "completed",
                "extra_object": {"n": null}
            },
            {
                "id": "r2",
                "summary": [{"text": "from-content", "type": "summary_text"}],
                "type": "reasoning",
                "reasoning_content": [{"type": "summary_text", "text": "from-content"}]
            },
            {"type": "message", "role": "user", "status": null}
        ]}"#,
    );
}

fn assert_reasoning_fallbacks() {
    assert_prepared(
        r#"{"model": "gpt-4o", "input": [
            {"type": "reasoning", "id": "r4", "reasoning_content": "a-string"},
            {"type": "reasoning", "id": "r6", "summary": "not-a-list", "status": null, "content": null, "encrypted_content": null, "kept_null": null},
            {"type": "reasoning", "id": 7, "summary": []},
            {"type": "reasoning", "id": "r8"},
            {"type": "reasoning", "id": "r9", "reasoning_content": null}
        ], "params": {}}"#,
        r#"{"model": "gpt-4o", "input": [
            {"type": "reasoning", "id": "r4", "reasoning_content": "a-string"},
            {"type": "reasoning", "id": "r6", "summary": "not-a-list", "kept_null": null},
            {"type": "reasoning", "id": 7, "summary": []},
            {"type": "reasoning", "id": "r8"},
            {"type": "reasoning", "id": "r9", "reasoning_content": null}
        ]}"#,
    );
}

fn assert_reasoning_content_limit() {
    let hundred = Value::Array(vec![fixture(SUMMARY_ENTRY); 100]);
    let mut at_limit = fixture(
        r#"{"model": "gpt-4o", "input": [{"type": "reasoning", "id": "r", "reasoning_content": []}], "params": {}}"#,
    );
    at_limit["input"][0]["reasoning_content"] = hundred.clone();
    let kept = expect_body(at_limit);
    assert_eq!(kept["input"][0]["summary"], hundred);
    assert_eq!(kept["input"][0]["reasoning_content"], hundred);

    let over = Value::Array(vec![fixture(SUMMARY_ENTRY); 101]);
    let mut past_limit = fixture(
        r#"{"model": "gpt-4o", "input": [{"type": "reasoning", "id": "r", "reasoning_content": []}], "params": {}}"#,
    );
    past_limit["input"][0]["reasoning_content"] = over.clone();
    let dropped = expect_body(past_limit);
    assert_eq!(dropped["input"][0].get("summary"), None);
    assert_eq!(dropped["input"][0]["reasoning_content"], over);
}

fn reasoning_summary_and_null() {
    assert_reasoning_validated();
    assert_reasoning_fallbacks();
    assert_reasoning_content_limit();
}

fn gpt4_schema_combinators() {
    assert_flattened("gpt-4o");
    assert_flattened("gpt-4.1");
    assert_flattened("gpt-3.5-turbo");
    assert_flattened("chatgpt-4o-latest");
    assert_flattened("o1-pro");
    assert_flattened("o3-mini");
    assert_flattened("o4-mini");
    assert_flattened("ft:gpt-4o:org::abc");
    assert_flattened("openai/gpt-4o");
}

fn gpt5_keeps_combinators() {
    assert_combinators_kept("gpt-5");
    assert_combinators_kept("ft:gpt-5:org::abc");
    assert_combinators_kept("my-deployment");
    assert_combinators_kept("ft:");
    assert_eq!(
        expect_body(provider_combiner_request("gpt-4o", "hosted_vllm")),
        schema_body("gpt-4o", COMBINER)
    );
    assert_eq!(
        expect_body(provider_combiner_request("my-deployment", "azure")),
        schema_body("my-deployment", KEPT_WITH_REGEX_DROPPED)
    );
}

fn base_model_gate() {
    assert_gate("my-deployment", r#"{"base_model": "gpt-4o"}"#, FLATTENED);
    assert_gate(
        "my-deployment",
        r#"{"base_model": "gpt-5"}"#,
        KEPT_WITH_REGEX_DROPPED,
    );
    assert_gate(
        "my-deployment",
        r#"{"base_model": "openai/gpt-4o"}"#,
        FLATTENED,
    );
    assert_gate(
        "my-deployment",
        r#"{"base_model": ""}"#,
        KEPT_WITH_REGEX_DROPPED,
    );
    assert_gate("gpt-4o", r#"{"base_model": ""}"#, FLATTENED);
    assert_gate(
        "my-deployment",
        r#"{"base_model": 5}"#,
        KEPT_WITH_REGEX_DROPPED,
    );
    assert_gate(
        "my-deployment",
        r#"{"base_model": null}"#,
        KEPT_WITH_REGEX_DROPPED,
    );
    assert_gate("my-deployment", r#"{"other": 1}"#, KEPT_WITH_REGEX_DROPPED);
    assert_gate("my-deployment", "null", KEPT_WITH_REGEX_DROPPED);
}

fn replay_body_overlay() {
    assert_prepared(
        r#"{"model": "gpt-4o", "input": "hi", "params": {}}"#,
        r#"{"model": "gpt-4o", "input": "hi"}"#,
    );
    assert_prepared(
        r#"{"model": "gpt-4o", "input": [{"role": "user", "content": "x"}], "params": {"temperature": 1, "stream": true, "metadata": {"a": 1}}}"#,
        r#"{"model": "gpt-4o", "input": [{"role": "user", "content": "x"}], "temperature": 1, "stream": true, "metadata": {"a": 1}}"#,
    );
    assert_prepared(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"tools": []}}"#,
        r#"{"model": "gpt-4o", "input": "hi", "tools": []}"#,
    );
    assert_prepared(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"tools": null}}"#,
        r#"{"model": "gpt-4o", "input": "hi", "tools": null}"#,
    );
    assert_prepared(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"tool_choice": "auto", "tools": [{"type": "function", "name": "g", "parameters": {"type": "object"}}]}}"#,
        r#"{"model": "gpt-4o", "input": "hi", "tool_choice": "auto", "tools": [{"type": "function", "name": "g", "parameters": {"type": "object"}}]}"#,
    );
    assert_overlay_duplicate_keys();
    assert_overlay_tools_shape();
}

fn assert_overlay_duplicate_keys() {
    let repeated = probe(fixture(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"model": "other"}}"#,
    ));
    let original = repeated.params.clone();
    let model_error = run(&repeated).expect_err("a repeated model key is rejected");
    assert_eq!(repeated.params, original);
    let ResponseInputError::DuplicateBodyKey { key: model_key } = model_error else {
        panic!("expected the duplicate key error");
    };
    assert_eq!(model_key, "model");
    let input_error = run(&probe(fixture(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"input": "other"}}"#,
    )))
    .expect_err("a repeated input key is rejected");
    let ResponseInputError::DuplicateBodyKey { key: input_key } = input_error else {
        panic!("expected the duplicate key error");
    };
    assert_eq!(input_key, "input");
    let prepared_first = expect_tool_error(fixture(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"model": "other", "tools": [{"parameters": 5}]}}"#,
    ));
    assert_eq!(prepared_first.http_status(), Some(400));
}

fn assert_overlay_tools_shape() {
    let text_tools = run(&probe(fixture(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"tools": "abc"}}"#,
    )))
    .expect_err("a string tools value is rejected");
    assert_eq!(text_tools.http_status(), Some(400));
    assert!(text_tools.to_string().contains("'tools'"));
    let map_tools = run(&probe(fixture(
        r#"{"model": "gpt-4o", "input": "hi", "params": {"tools": {"a": 1}}}"#,
    )))
    .expect_err("an object tools value is rejected");
    assert_eq!(map_tools.http_status(), Some(400));
    assert!(map_tools.to_string().contains("'tools'"));
}

#[rstest]
#[case::case_1_string_schema_object(string_schema_object)]
#[case::case_2_invalid_string_schema(invalid_string_schema)]
#[case::case_3_recursive_tool_namespace(recursive_tool_namespace)]
#[case::case_4_foreign_tool_item_id(foreign_tool_item_id)]
#[case::case_5_cache_control_in_input_and_tools(cache_control_in_input_and_tools)]
#[case::case_6_reasoning_summary_and_null(reasoning_summary_and_null)]
#[case::case_7_gpt4_schema_combinators(gpt4_schema_combinators)]
#[case::case_8_gpt5_keeps_combinators(gpt5_keeps_combinators)]
#[case::case_9_base_model_gate(base_model_gate)]
#[case::case_10_replay_body_overlay(replay_body_overlay)]
fn response_input_cases(#[case] scenario: fn()) {
    scenario();
}
