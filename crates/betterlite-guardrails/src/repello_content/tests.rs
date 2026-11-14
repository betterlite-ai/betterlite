#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::SUPPORTED_EVENT_HOOKS;
use super::config::RepelloAIGuardrailConfigModel;
use super::prompt::{extract_prompt_text, input_text_parts, prompt_field_text};
use super::response::{
    ResponseSource, chat_completion_text, extract_response_text, responses_api_text,
};
use super::schema::{schema_text, tool_definition_text};
use super::tool_calls::tool_call_args;
use super::wire::{
    RepelloAIAnalyzeRequest, RepelloAIAnalyzeResponse, RepelloAIScanData, RepelloAIViolatedPolicy,
};
use crate::generic_guardrail_contracts::UnreachableFallback;

fn data(source: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(source).expect("the fixture is a JSON object")
}

fn value(source: &str) -> GuardrailValue {
    serde_json::from_str(source).expect("the fixture is JSON")
}

fn prompt(source: &str) -> Option<String> {
    extract_prompt_text(&data(source))
}

#[test]
fn supported_event_hooks_are_pre_and_post_call() {
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall]
    );
}

#[test]
fn config_model_defaults_every_missing_field() {
    let model: RepelloAIGuardrailConfigModel =
        serde_json::from_str("{}").expect("an empty config model");
    assert!(model.api_key.is_none());
    assert_eq!(model.api_base, None);
    assert_eq!(model.asset_id, None);
    assert_eq!(model.unreachable_fallback, UnreachableFallback::FailClosed);
    assert!(model.optional_params.is_none());
}

#[test]
fn config_model_reads_every_field() {
    let model: RepelloAIGuardrailConfigModel = serde_json::from_str(
        r#"{"api_key":"argus-key","api_base":"https://argus.test/sdk/v1","asset_id":"asset-1","unreachable_fallback":"fail_open","optional_params":{"score":0.5}}"#,
    )
    .expect("the full config model");
    assert_eq!(
        model.api_key.as_ref().expect("the api key").expose_secret(),
        "argus-key"
    );
    assert_eq!(model.api_base.as_deref(), Some("https://argus.test/sdk/v1"));
    assert_eq!(model.asset_id.as_deref(), Some("asset-1"));
    assert_eq!(model.unreachable_fallback, UnreachableFallback::FailOpen);
    let optional = model.optional_params.expect("the optional params");
    assert_eq!(optional.len(), 1);
    assert!(optional.contains_key("score"));
}

#[rstest]
#[case::fail_closed("fail_closed", UnreachableFallback::FailClosed)]
#[case::fail_open("fail_open", UnreachableFallback::FailOpen)]
fn config_model_reads_either_unreachable_fallback(
    #[case] raw: &str,
    #[case] expected: UnreachableFallback,
) {
    let source = format!(r#"{{"unreachable_fallback":"{raw}"}}"#);
    let model: RepelloAIGuardrailConfigModel =
        serde_json::from_str(&source).expect("the unreachable fallback");
    assert_eq!(model.unreachable_fallback, expected);
}

#[test]
fn config_model_rejects_an_unknown_unreachable_fallback() {
    let model = serde_json::from_str::<RepelloAIGuardrailConfigModel>(
        r#"{"unreachable_fallback":"allow"}"#,
    );
    assert!(model.is_err());
}

#[test]
fn config_model_rejects_an_unknown_field() {
    let model =
        serde_json::from_str::<RepelloAIGuardrailConfigModel>(r#"{"asset_id":"a","extra":true}"#);
    assert!(model.is_err());
}

#[test]
fn config_model_ui_friendly_name_is_repelloai_argus() {
    assert_eq!(
        RepelloAIGuardrailConfigModel::ui_friendly_name(),
        "RepelloAI Argus"
    );
}

#[test]
fn scan_data_serializes_only_the_prompt_stage() {
    let scan = RepelloAIScanData {
        prompt: Some("hello".to_owned()),
        response: None,
    };
    assert_eq!(
        serde_json::to_string(&scan).expect("the scan data"),
        r#"{"prompt":"hello"}"#
    );
}

#[test]
fn scan_data_serializes_only_the_response_stage() {
    let scan = RepelloAIScanData {
        prompt: None,
        response: Some("reply".to_owned()),
    };
    assert_eq!(
        serde_json::to_string(&scan).expect("the scan data"),
        r#"{"response":"reply"}"#
    );
}

#[test]
fn scan_data_serializes_an_empty_object_without_a_stage() {
    let scan = RepelloAIScanData::default();
    assert_eq!(serde_json::to_string(&scan).expect("the scan data"), "{}");
}

#[test]
fn scan_data_reads_a_stage_from_the_wire() {
    let scan: RepelloAIScanData =
        serde_json::from_str(r#"{"response":"reply"}"#).expect("the scan data");
    assert_eq!(scan.prompt, None);
    assert_eq!(scan.response.as_deref(), Some("reply"));
}

#[test]
fn analyze_request_serializes_asset_id_and_scan_data() {
    let request = RepelloAIAnalyzeRequest {
        asset_id: "asset-1".to_owned(),
        scan_data: RepelloAIScanData {
            prompt: Some("hello".to_owned()),
            response: None,
        },
    };
    assert_eq!(
        serde_json::to_string(&request).expect("the analyze request"),
        r#"{"asset_id":"asset-1","scan_data":{"prompt":"hello"}}"#
    );
}

#[test]
fn analyze_response_reads_every_field() {
    let response: RepelloAIAnalyzeResponse = serde_json::from_str(
        r#"{"verdict":"blocked","request_id":"req-1","policies_violated":[{"policy_name":"pii","policy_id":"p1","action_taken":"block","scope":"input","details":{"score":0.9},"masked_result":"***"}],"policies_applied":[{"policy_name":"spam"}]}"#,
    )
    .expect("the analyze response");
    assert_eq!(response.verdict.as_deref(), Some("blocked"));
    assert_eq!(response.request_id.as_deref(), Some("req-1"));
    let violated = response.policies_violated.expect("the violated policies");
    assert_eq!(violated.len(), 1);
    let policy = &violated[0];
    assert_eq!(policy.policy_name.as_deref(), Some("pii"));
    assert_eq!(policy.policy_id.as_deref(), Some("p1"));
    assert_eq!(policy.action_taken.as_deref(), Some("block"));
    assert_eq!(policy.scope.as_deref(), Some("input"));
    assert_eq!(policy.masked_result.as_deref(), Some("***"));
    let details = policy.details.as_ref().expect("the policy details");
    assert_eq!(details.len(), 1);
    assert!(details.contains_key("score"));
    let applied = response.policies_applied.expect("the applied policies");
    assert_eq!(applied.len(), 1);
    assert!(applied[0].contains_key("policy_name"));
}

#[test]
fn analyze_response_defaults_missing_fields_to_none() {
    let response: RepelloAIAnalyzeResponse =
        serde_json::from_str("{}").expect("an empty analyze response");
    assert!(response.verdict.is_none());
    assert!(response.request_id.is_none());
    assert!(response.policies_violated.is_none());
    assert!(response.policies_applied.is_none());
}

#[test]
fn analyze_response_ignores_fields_it_does_not_model() {
    let response: RepelloAIAnalyzeResponse =
        serde_json::from_str(r#"{"verdict":"passed","latency_ms":5}"#).expect("the extra field");
    assert_eq!(response.verdict.as_deref(), Some("passed"));
}

#[test]
fn violated_policy_keeps_an_open_details_map() {
    let policy: RepelloAIViolatedPolicy =
        serde_json::from_str(r#"{"details":{"score":0.5,"labels":["a","b"]}}"#)
            .expect("the policy");
    let details = policy.details.expect("the details");
    assert_eq!(details.len(), 2);
    assert!(details.contains_key("labels"));
}

#[rstest]
#[case::name(r#"{"name":"n"}"#, "n")]
#[case::description(r#"{"description":"d"}"#, "d")]
#[case::title(r#"{"title":"t"}"#, "t")]
#[case::constant(r#"{"const":"c"}"#, "c")]
#[case::default_value(r#"{"default":"v"}"#, "v")]
fn schema_text_reads_every_scalar_key(#[case] schema: &str, #[case] expected: &str) {
    assert_eq!(schema_text(&data(schema)), [expected]);
}

#[rstest]
#[case::empty_name(r#"{"name":""}"#)]
#[case::null_description(r#"{"description":null}"#)]
#[case::numeric_title(r#"{"title":3}"#)]
#[case::list_constant(r#"{"const":["c"]}"#)]
#[case::object_default(r#"{"default":{"nested":"d"}}"#)]
fn schema_text_skips_non_string_scalars(#[case] schema: &str) {
    assert!(schema_text(&data(schema)).is_empty());
}

#[rstest]
#[case::enum_list(r#"{"enum":["a","b"]}"#, &["a", "b"])]
#[case::examples_list(r#"{"examples":["x"]}"#, &["x"])]
fn schema_text_reads_list_keys(#[case] schema: &str, #[case] expected: &[&str]) {
    assert_eq!(schema_text(&data(schema)), expected);
}

#[rstest]
#[case::empty_items_skipped(r#"{"enum":["a","",null,3]}"#, &["a"])]
#[case::not_a_list(r#"{"enum":"a"}"#, &[])]
#[case::empty_list(r#"{"enum":[]}"#, &[])]
#[case::objects_in_the_list_are_not_walked(r#"{"enum":[{"name":"hidden"}]}"#, &[])]
fn schema_text_keeps_only_non_empty_string_items(#[case] schema: &str, #[case] expected: &[&str]) {
    assert_eq!(schema_text(&data(schema)), expected);
}

#[test]
fn schema_text_walks_nested_objects_before_a_later_sibling() {
    let schema = data(
        r#"{"name":"outer","parameters":{"properties":{"q":{"description":"inner"}}},"title":"after"}"#,
    );
    assert_eq!(schema_text(&schema), ["outer", "after", "inner"]);
}

#[test]
fn schema_text_walks_depth_first_across_siblings() {
    let schema = data(r#"{"a":{"inner":{"name":"deep"},"name":"a1"},"b":{"name":"b1"}}"#);
    assert_eq!(schema_text(&schema), ["a1", "deep", "b1"]);
}

#[test]
fn schema_text_walks_arrays_in_source_order() {
    let schema = data(r#"{"anyOf":[{"title":"first"},{"title":"second"}]}"#);
    assert_eq!(schema_text(&schema), ["first", "second"]);
}

#[test]
fn schema_text_walks_sibling_objects_in_key_order() {
    let schema = data(r#"{"zeta":{"name":"z"},"alpha":{"name":"a"}}"#);
    assert_eq!(schema_text(&schema), ["a", "z"]);
}

#[test]
fn schema_text_does_not_descend_into_extracted_keys() {
    let schema = data(r#"{"name":{"nested":"hidden"},"parameters":{"description":"shown"}}"#);
    assert_eq!(schema_text(&schema), ["shown"]);
}

#[test]
fn schema_text_ignores_scalars_it_does_not_know() {
    let schema = data(r#"{"type":"object","format":"date","minimum":1,"enabled":true}"#);
    assert!(schema_text(&schema).is_empty());
}

#[test]
fn tool_definition_text_reads_tools_and_functions() {
    let source = data(
        r#"{"tools":[{"type":"function","function":{"name":"tool"}}],"functions":[{"name":"legacy"}]}"#,
    );
    assert_eq!(tool_definition_text(&source), ["tool", "legacy"]);
}

#[rstest]
#[case::tool_without_function(r#"{"tools":[{"type":"function"}]}"#)]
#[case::tool_that_is_a_string(r#"{"tools":["tool"]}"#)]
#[case::function_that_is_a_string(r#"{"tools":[{"function":"tool"}]}"#)]
#[case::function_entry_that_is_a_string(r#"{"functions":["legacy"]}"#)]
#[case::tools_not_a_list(r#"{"tools":{"name":"tool"}}"#)]
#[case::functions_not_a_list(r#"{"functions":{"name":"legacy"}}"#)]
#[case::missing_both(r#"{"name":"plain"}"#)]
fn tool_definition_text_skips_entries_without_an_object_schema(#[case] source: &str) {
    assert!(tool_definition_text(&data(source)).is_empty());
}

#[test]
fn tool_call_args_reads_tool_calls_then_the_legacy_function_call() {
    let message = value(
        r#"{"tool_calls":[{"function":{"arguments":"{\"a\":1}"}},{"function":{"arguments":"second"}}],"function_call":{"arguments":"legacy"}}"#,
    );
    assert_eq!(tool_call_args(&message), ["{\"a\":1}", "second", "legacy"]);
}

#[rstest]
#[case::blank_arguments(r#"{"tool_calls":[{"function":{"arguments":"   "}}]}"#)]
#[case::empty_arguments(r#"{"tool_calls":[{"function":{"arguments":""}}]}"#)]
#[case::numeric_arguments(r#"{"tool_calls":[{"function":{"arguments":5}}]}"#)]
#[case::function_without_arguments(r#"{"tool_calls":[{"function":{}}]}"#)]
#[case::tool_call_that_is_a_string(r#"{"tool_calls":["call"]}"#)]
#[case::tool_calls_not_a_list(r#"{"tool_calls":{"function":{"arguments":"x"}}}"#)]
#[case::blank_function_call(r#"{"function_call":{"arguments":" "}}"#)]
#[case::function_call_that_is_a_string(r#"{"function_call":"call"}"#)]
#[case::null_fields(r#"{"tool_calls":null,"function_call":null}"#)]
#[case::missing_fields(r#"{"role":"assistant"}"#)]
fn tool_call_args_skips_blank_and_malformed_arguments(#[case] message: &str) {
    assert!(tool_call_args(&value(message)).is_empty());
}

#[test]
fn tool_call_args_keeps_the_arguments_verbatim() {
    let message = value(r#"{"function_call":{"arguments":"  padded  "}}"#);
    assert_eq!(tool_call_args(&message), ["  padded  "]);
}

#[rstest]
#[case::input_text_part(r#"[{"type":"input_text","text":"a"}]"#, &["a"])]
#[case::other_types_skipped(
    r#"[{"type":"text","text":"a"},{"type":"output_text","text":"b"}]"#,
    &[]
)]
#[case::only_input_text_parts_kept(
    r#"[{"type":"input_text","text":"a"},{"type":"text","text":"b"}]"#,
    &["a"]
)]
#[case::empty_text_skipped(r#"[{"type":"input_text","text":""}]"#, &[])]
#[case::non_string_text_skipped(r#"[{"type":"input_text","text":3}]"#, &[])]
#[case::non_string_type_skipped(r#"[{"type":5,"text":"a"}]"#, &[])]
#[case::missing_type_skipped(r#"[{"text":"a"}]"#, &[])]
#[case::string_parts_skipped(r#"["a"]"#, &[])]
#[case::not_a_list(r#"{"type":"input_text","text":"a"}"#, &[])]
#[case::null_content("null", &[])]
fn input_text_parts_reads_only_input_text_parts(#[case] content: &str, #[case] expected: &[&str]) {
    let parsed = value(content);
    assert_eq!(input_text_parts(Some(&parsed)), expected);
}

#[rstest]
#[case::string(r#"{"prompt":"hello"}"#, &["hello"])]
#[case::empty_string(r#"{"prompt":""}"#, &[])]
#[case::whitespace(r#"{"prompt":"  "}"#, &["  "])]
#[case::string_list(r#"{"prompt":["a","","b"]}"#, &["a", "b"])]
#[case::list_with_non_strings(r#"{"prompt":["a",1,null,["b"]]}"#, &["a"])]
#[case::number(r#"{"prompt":7}"#, &[])]
#[case::object(r#"{"prompt":{"text":"a"}}"#, &[])]
#[case::missing("{}", &[])]
fn prompt_field_text_reads_a_string_or_a_string_list(
    #[case] source: &str,
    #[case] expected: &[&str],
) {
    assert_eq!(prompt_field_text(&data(source)), expected);
}

#[test]
fn prompt_text_returns_none_without_any_text() {
    assert_eq!(prompt("{}"), None);
    assert_eq!(prompt(r#"{"messages":[]}"#), None);
    assert_eq!(prompt(r#"{"prompt":""}"#), None);
    assert_eq!(prompt(r#"{"input":[]}"#), None);
}

#[test]
fn prompt_text_scans_message_content() {
    assert_eq!(
        prompt(r#"{"messages":[{"role":"user","content":"hello"}]}"#),
        Some("hello".to_owned())
    );
}

#[test]
fn prompt_text_scans_multimodal_message_content() {
    assert_eq!(
        prompt(
            r#"{"messages":[{"role":"user","content":[{"type":"text","text":"a"},{"type":"image_url","image_url":{"url":"u"}},{"type":"text","text":"b"}]}]}"#
        ),
        Some("a\nb".to_owned())
    );
}

#[test]
fn prompt_text_scans_a_string_input() {
    assert_eq!(prompt(r#"{"input":"hi"}"#), Some("hi".to_owned()));
}

#[test]
fn prompt_text_reads_the_prompt_field() {
    assert_eq!(
        prompt(r#"{"prompt":"legacy prompt"}"#),
        Some("legacy prompt".to_owned())
    );
}

#[test]
fn prompt_text_reads_instructions() {
    assert_eq!(
        prompt(r#"{"instructions":"be brief"}"#),
        Some("be brief".to_owned())
    );
    assert_eq!(prompt(r#"{"instructions":""}"#), None);
    assert_eq!(prompt(r#"{"instructions":["be brief"]}"#), None);
}

#[test]
fn prompt_text_reads_tool_call_arguments_from_messages() {
    assert_eq!(
        prompt(
            r#"{"messages":[{"role":"assistant","tool_calls":[{"function":{"arguments":"first"}}],"function_call":{"arguments":"legacy"}}]}"#
        ),
        Some("first\nlegacy".to_owned())
    );
}

#[test]
fn prompt_text_reads_tool_call_arguments_only_from_input_items_with_role() {
    let with_role =
        r#"{"input":[{"role":"user","tool_calls":[{"function":{"arguments":"args"}}]}]}"#;
    assert_eq!(prompt(with_role), Some("args".to_owned()));
    let without_role = r#"{"input":[{"tool_calls":[{"function":{"arguments":"args"}}]}]}"#;
    assert_eq!(prompt(without_role), None);
}

#[test]
fn prompt_text_scans_input_text_parts_twice_for_role_items() {
    assert_eq!(
        prompt(r#"{"input":[{"role":"user","content":[{"type":"input_text","text":"part"}]}]}"#),
        Some("part\npart".to_owned())
    );
}

#[test]
fn prompt_text_reads_tool_and_function_schemas() {
    assert_eq!(
        prompt(
            r#"{"tools":[{"type":"function","function":{"name":"tool_name","description":"tool desc"}}],"functions":[{"name":"legacy_fn"}]}"#
        ),
        Some("tool_name\ntool desc\nlegacy_fn".to_owned())
    );
}

#[test]
fn prompt_text_joins_every_source_in_litellm_order() {
    let source = r#"{
        "messages": [
            {"role": "system", "content": "system rules"},
            {"role": "assistant", "content": "answer", "tool_calls": [{"function": {"arguments": "{\"a\":1}"}}, {"function": {"arguments": "{\"b\":2}"}}], "function_call": {"arguments": "{\"c\":3}"}}
        ],
        "prompt": "legacy prompt",
        "instructions": "be brief",
        "input": [
            {"role": "user", "content": [{"type": "input_text", "text": "input part"}]},
            {"type": "function_call", "arguments": "{\"d\":4}"}
        ],
        "tools": [{"type": "function", "function": {"name": "tool_name", "description": "tool desc"}}],
        "functions": [{"name": "legacy_fn"}]
    }"#;
    assert_eq!(
        prompt(source),
        Some(
            "system rules\nanswer\ninput part\nlegacy prompt\nbe brief\n{\"a\":1}\n{\"b\":2}\n{\"c\":3}\ninput part\ntool_name\ntool desc\nlegacy_fn"
                .to_owned()
        )
    );
}

#[test]
fn chat_completion_text_reads_message_content() {
    assert_eq!(
        chat_completion_text(&data(r#"{"choices":[{"message":{"content":"a"}}]}"#)),
        Some("a".to_owned())
    );
}

#[test]
fn chat_completion_text_reads_choice_text() {
    assert_eq!(
        chat_completion_text(&data(r#"{"choices":[{"text":"t"}]}"#)),
        Some("t".to_owned())
    );
}

#[test]
fn chat_completion_text_reads_message_content_before_choice_text() {
    assert_eq!(
        chat_completion_text(&data(
            r#"{"choices":[{"message":{"content":"a","tool_calls":[{"function":{"arguments":"x"}}],"function_call":{"arguments":"y"}},"text":"t"}]}"#
        )),
        Some("a\nx\ny\nt".to_owned())
    );
}

#[test]
fn chat_completion_text_joins_choices_with_newlines() {
    assert_eq!(
        chat_completion_text(&data(
            r#"{"choices":[{"message":{"content":"a"}},{"message":{"content":"b"}}]}"#
        )),
        Some("a\nb".to_owned())
    );
}

#[rstest]
#[case::skips_non_object_choices(r#"{"choices":["x",{"text":"t"}]}"#, Some("t"))]
#[case::skips_empty_content_and_text(r#"{"choices":[{"message":{"content":""},"text":""}]}"#, None)]
#[case::skips_a_non_object_message(r#"{"choices":[{"message":"m","text":"t"}]}"#, Some("t"))]
fn chat_completion_text_keeps_the_text_it_finds(
    #[case] source: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(chat_completion_text(&data(source)).as_deref(), expected);
}

#[rstest]
#[case::missing_choices("{}")]
#[case::choices_not_a_list(r#"{"choices":{"message":{"content":"a"}}}"#)]
#[case::empty_choices(r#"{"choices":[]}"#)]
fn chat_completion_text_returns_none_without_text(#[case] source: &str) {
    assert_eq!(chat_completion_text(&data(source)), None);
}

#[test]
fn responses_api_text_reads_function_call_arguments() {
    assert_eq!(
        responses_api_text(&data(
            r#"{"output":[{"type":"function_call","arguments":"{\"q\":1}"}]}"#
        )),
        Some("{\"q\":1}".to_owned())
    );
}

#[test]
fn responses_api_text_reads_message_text_parts() {
    assert_eq!(
        responses_api_text(&data(
            r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"a"},{"type":"text","text":"b"}]}]}"#
        )),
        Some("ab".to_owned())
    );
}

#[test]
fn responses_api_text_concatenates_output_items_without_a_separator() {
    assert_eq!(
        responses_api_text(&data(
            r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"a"}]},{"type":"function_call","arguments":"b"},{"type":"message","content":[{"type":"text","text":"c"}]}]}"#
        )),
        Some("abc".to_owned())
    );
}

#[rstest]
#[case::other_item_types(
    r#"{"output":[{"type":"reasoning","content":[{"type":"output_text","text":"hidden"}]}]}"#
)]
#[case::other_part_types(
    r#"{"output":[{"type":"message","content":[{"type":"input_text","text":"hidden"}]}]}"#
)]
#[case::empty_parts(
    r#"{"output":[{"type":"message","content":[{"type":"output_text","text":""}]}]}"#
)]
#[case::empty_arguments(r#"{"output":[{"type":"function_call","arguments":""}]}"#)]
#[case::string_output_items(r#"{"output":["text"]}"#)]
#[case::message_without_content_list(r#"{"output":[{"type":"message","content":"text"}]}"#)]
fn responses_api_text_skips_what_litellm_skips(#[case] source: &str) {
    assert_eq!(responses_api_text(&data(source)), None);
}

#[rstest]
#[case::missing_output("{}")]
#[case::output_not_a_list(r#"{"output":{"type":"message"}}"#)]
#[case::empty_output(r#"{"output":[]}"#)]
fn responses_api_text_returns_none_without_output(#[case] source: &str) {
    assert_eq!(responses_api_text(&data(source)), None);
}

#[test]
fn response_text_prefers_chat_completion_over_responses_output() {
    assert_eq!(
        extract_response_text(&ResponseSource::Body(&data(
            r#"{"choices":[{"message":{"content":"chat"}}],"output":[{"type":"message","content":[{"type":"output_text","text":"api"}]}]}"#
        ))),
        Some("chat".to_owned())
    );
}

#[test]
fn response_text_falls_back_to_responses_output() {
    assert_eq!(
        extract_response_text(&ResponseSource::Body(&data(
            r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"api"}]}]}"#
        ))),
        Some("api".to_owned())
    );
}

#[test]
fn response_text_returns_none_for_a_body_without_text() {
    assert_eq!(
        extract_response_text(&ResponseSource::Body(&data(r#"{"id":"x"}"#))),
        None
    );
}

#[test]
fn response_text_reads_output_text_from_a_typed_response() {
    assert_eq!(
        extract_response_text(&ResponseSource::OutputText("typed")),
        Some("typed".to_owned())
    );
}

#[test]
fn response_text_ignores_an_empty_output_text() {
    assert_eq!(extract_response_text(&ResponseSource::OutputText("")), None);
}

#[test]
fn response_text_does_not_read_an_output_text_key_from_a_body() {
    assert_eq!(
        extract_response_text(&ResponseSource::Body(&data(r#"{"output_text":"typed"}"#))),
        None
    );
}
