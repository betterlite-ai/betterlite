#![cfg(test)]

use std::fs;
use std::iter::repeat_n;
use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rstest::rstest;

use super::error::GeminiToolInvokesError;
use super::invoke::{convert_tool_calls_to_gemini, gemini_tool_call_invoke_helper};
use super::limits::{MAX_MESSAGE_BYTES, MAX_TOOL_CALLS};
use super::parser::parse_tool_invoke_message;
use super::protocol::{ProtocolValue, infer_protocol_value};
use super::signatures::{
    DUMMY_THOUGHT_SIGNATURE, encode_tool_call_id, thought_signature_from_tool,
};
use super::types::{GeminiFunctionCallParams, GeminiTool};
use crate::arguments::json::Json;
use crate::message_content::Message;

enum Failure {
    MessageDecode,
    Fields(&'static str),
    Arguments,
}

struct ConvertCase {
    message: &'static str,
    model: Option<&'static str>,
    forward: bool,
    expected: &'static str,
}

struct ConvertFailureCase {
    message: &'static str,
    model: Option<&'static str>,
    forward: bool,
    failure: Failure,
}

struct HelperCase {
    params: &'static str,
    tool_call_id: Option<&'static str>,
    expected: &'static str,
}

struct HelperFailureCase {
    params: &'static str,
    tool_call_id: Option<&'static str>,
    failure: Failure,
}

struct EncodeCase {
    tool_call_id: &'static str,
    thought_signature: Option<&'static str>,
    expected: &'static str,
}

struct SignatureCase {
    tool: &'static str,
    expected: Option<&'static str>,
}

struct ProtocolCase {
    value: &'static str,
    expected: ProtocolValue,
}

enum Scenario {
    Convert(ConvertCase),
    ConvertFailure(ConvertFailureCase),
    ParseFailure(&'static str),
    Helper(HelperCase),
    HelperFailure(HelperFailureCase),
    Encode(EncodeCase),
    Signature(SignatureCase),
    Protocol(ProtocolCase),
    Dummy,
    Preserved(&'static str),
    OversizedMessage,
    TooManyToolCalls,
}

fn parsed_json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn parse_message(text: &str) -> Message {
    parse_tool_invoke_message(text.as_bytes()).unwrap()
}

fn assert_parts(case: &ConvertCase) {
    let converted =
        convert_tool_calls_to_gemini(&parse_message(case.message), case.model, case.forward)
            .unwrap();
    assert_eq!(
        serde_json::to_value(&converted).unwrap(),
        serde_json::to_value(parsed_json(case.expected)).unwrap()
    );
}

fn assert_convert_failure(case: &ConvertFailureCase) {
    let error =
        convert_tool_calls_to_gemini(&parse_message(case.message), case.model, case.forward)
            .unwrap_err();
    assert_failure(&error, &case.failure);
}

fn assert_parse_failure(message: &str) {
    let error = parse_tool_invoke_message(message.as_bytes()).unwrap_err();
    assert_failure(&error, &Failure::MessageDecode);
}

fn assert_helper(case: &HelperCase) {
    let parsed_params: GeminiFunctionCallParams = serde_json::from_str(case.params).unwrap();
    let call = gemini_tool_call_invoke_helper(&parsed_params, case.tool_call_id).unwrap();
    assert_eq!(
        serde_json::to_value(&call).unwrap(),
        serde_json::to_value(parsed_json(case.expected)).unwrap()
    );
}

fn assert_helper_failure(case: &HelperFailureCase) {
    let parsed_params: GeminiFunctionCallParams = serde_json::from_str(case.params).unwrap();
    let error = gemini_tool_call_invoke_helper(&parsed_params, case.tool_call_id).unwrap_err();
    assert_failure(&error, &case.failure);
}

fn assert_failure(error: &GeminiToolInvokesError, failure: &Failure) {
    match failure {
        Failure::MessageDecode => {
            assert!(matches!(
                error,
                GeminiToolInvokesError::MessageDecode { .. }
            ));
        }
        Failure::Arguments => {
            assert!(matches!(error, GeminiToolInvokesError::Arguments { .. }));
        }
        Failure::Fields(path) => {
            assert!(matches!(error, GeminiToolInvokesError::Fields { .. }));
            assert!(
                error.to_string().contains(path),
                "{error} does not name {path}"
            );
        }
    }
}

fn assert_signature(case: &SignatureCase) {
    let parsed_tool: GeminiTool = serde_json::from_str(case.tool).unwrap();
    assert_eq!(
        thought_signature_from_tool(&parsed_tool).as_deref(),
        case.expected
    );
}

fn assert_preserved(message: &str) {
    let parsed = parse_message(message);
    let untouched = parsed.clone();
    drop(convert_tool_calls_to_gemini(&parsed, Some("gemini-3-pro-preview"), true).unwrap());
    drop(convert_tool_calls_to_gemini(&parsed, None, false).unwrap());
    assert_eq!(parsed, untouched);
}

fn assert_message_too_large() {
    let bytes = vec![b' '; MAX_MESSAGE_BYTES.saturating_add(1)];
    let error = parse_tool_invoke_message(&bytes).unwrap_err();
    assert!(matches!(
        error,
        GeminiToolInvokesError::MessageTooLarge { length, limit }
            if length == MAX_MESSAGE_BYTES.saturating_add(1) && limit == MAX_MESSAGE_BYTES
    ));
}

fn assert_too_many_tool_calls() {
    let tool = r#"{"id":"call","function":{"name":"tool","arguments":"{}"}}"#;
    let calls = repeat_n(tool, MAX_TOOL_CALLS.saturating_add(1))
        .collect::<Vec<_>>()
        .join(",");
    let text = format!(r#"{{"role":"assistant","tool_calls":[{calls}]}}"#);
    let error = convert_tool_calls_to_gemini(&parse_message(&text), None, false).unwrap_err();
    assert!(matches!(
        error,
        GeminiToolInvokesError::TooManyToolCalls { limit } if limit == MAX_TOOL_CALLS
    ));
}

fn run(scenario: Scenario) {
    match scenario {
        Scenario::Convert(case) => assert_parts(&case),
        Scenario::ConvertFailure(case) => assert_convert_failure(&case),
        Scenario::ParseFailure(message) => assert_parse_failure(message),
        Scenario::Helper(case) => assert_helper(&case),
        Scenario::HelperFailure(case) => assert_helper_failure(&case),
        Scenario::Encode(case) => assert_eq!(
            encode_tool_call_id(case.tool_call_id, case.thought_signature),
            case.expected
        ),
        Scenario::Signature(case) => assert_signature(&case),
        Scenario::Protocol(case) => assert_eq!(
            infer_protocol_value(&parsed_json(case.value)),
            case.expected
        ),
        Scenario::Dummy => assert_eq!(
            STANDARD.encode(b"skip_thought_signature_validator"),
            DUMMY_THOUGHT_SIGNATURE
        ),
        Scenario::Preserved(message) => assert_preserved(message),
        Scenario::OversizedMessage => assert_message_too_large(),
        Scenario::TooManyToolCalls => assert_too_many_tool_calls(),
    }
}

#[rstest]
#[case::parallel_invokes_keep_order_and_clean_ids(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","content":null,"tool_calls":[
        {"id":"call_0__thought__sig-zero","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}},
        {"id":"call_1","type":"function","function":{"name":"get_time","arguments":""}},
        {"id":"call_2","type":"function","function":{"name":"get_stock","arguments":"{\"symbol\":\"ACME\"}","provider_specific_fields":{"thought_signature":"sig-two"}}}
    ]}"#,
    model: Some("gemini-3-pro-preview"),
    forward: true,
    expected: r#"[
        {"function_call":{"name":"get_weather","args":{"city":"Paris"},"id":"call_0"},"thoughtSignature":"sig-zero"},
        {"function_call":{"name":"get_time","args":{"type":"object"},"id":"call_1"}},
        {"function_call":{"name":"get_stock","args":{"symbol":"ACME"},"id":"call_2"},"thoughtSignature":"sig-two"}
    ]"#,
}))]
#[case::tool_level_signature_beats_function_level(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[{"id":"call_x__thought__id-sig","function":{"name":"n","arguments":"{}","provider_specific_fields":{"thought_signature":"function-sig"}},"provider_specific_fields":{"thought_signature":"tool-sig"}}]}"#,
    model: None,
    forward: true,
    expected: r#"[{"function_call":{"name":"n","args":{},"id":"call_x"},"thoughtSignature":"tool-sig"}]"#,
}))]
#[case::dummy_signature_lands_on_the_first_emitted_part(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[
        {"id":"call_custom","type":"custom","custom":{"name":"noop","input":""}},
        {"id":"call_a","function":{"name":"first","arguments":"{}"}},
        {"id":"call_b","function":{"name":"second","arguments":"{}"}}
    ]}"#,
    model: Some("gemini-3-pro-preview"),
    forward: false,
    expected: r#"[
        {"function_call":{"name":"first","args":{}},"thoughtSignature":"c2tpcF90aG91Z2h0X3NpZ25hdHVyZV92YWxpZGF0b3I="},
        {"function_call":{"name":"second","args":{}}}
    ]"#,
}))]
#[case::signed_first_call_gets_no_placeholder_sibling(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[
        {"id":"call_a","function":{"name":"first","arguments":"{}","provider_specific_fields":{"thought_signature":"real"}}},
        {"id":"call_b","function":{"name":"second","arguments":"{}"}}
    ]}"#,
    model: Some("gemini-3-pro-preview"),
    forward: false,
    expected: r#"[
        {"function_call":{"name":"first","args":{}},"thoughtSignature":"real"},
        {"function_call":{"name":"second","args":{}}}
    ]"#,
}))]
#[case::later_call_signature_is_preserved(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[
        {"id":"call_a","function":{"name":"first","arguments":"{}"}},
        {"id":"call_b","function":{"name":"second","arguments":"{}","provider_specific_fields":{"thought_signature":"later"}}}
    ]}"#,
    model: Some("gemini-3-pro-preview"),
    forward: false,
    expected: r#"[
        {"function_call":{"name":"first","args":{}},"thoughtSignature":"c2tpcF90aG91Z2h0X3NpZ25hdHVyZV92YWxpZGF0b3I="},
        {"function_call":{"name":"second","args":{}},"thoughtSignature":"later"}
    ]"#,
}))]
#[case::function_call_mode_gets_a_dummy_signature(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","function_call":{"name":"get_current_temperature","arguments":"{\"location\":\"Paris\"}"}}"#,
    model: Some("gemini-3-pro-preview"),
    forward: true,
    expected: r#"[{"function_call":{"name":"get_current_temperature","args":{"location":"Paris"}},"thoughtSignature":"c2tpcF90aG91Z2h0X3NpZ25hdHVyZV92YWxpZGF0b3I="}]"#,
}))]
#[case::function_call_keeps_its_provider_signature(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","function_call":{"name":"fn","arguments":"{}","provider_specific_fields":{"thought_signature":"fn-sig"}}}"#,
    model: Some("gemini-3-pro-preview"),
    forward: true,
    expected: r#"[{"function_call":{"name":"fn","args":{}},"thoughtSignature":"fn-sig"}]"#,
}))]
#[case::older_models_get_no_dummy_signature(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[{"id":"call_a","function":{"name":"first","arguments":"{}"}}]}"#,
    model: Some("gemini-2.5-flash"),
    forward: false,
    expected: r#"[{"function_call":{"name":"first","args":{}}}]"#,
}))]
#[case::unknown_model_gets_no_dummy_signature(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[{"id":"call_a","function":{"name":"first","arguments":"{}"}}]}"#,
    model: None,
    forward: false,
    expected: r#"[{"function_call":{"name":"first","args":{}}}]"#,
}))]
#[case::gemini_flash_latest_is_newer(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[{"function":{"name":"first","arguments":"{}"}}]}"#,
    model: Some("gemini-flash-latest"),
    forward: false,
    expected: r#"[{"function_call":{"name":"first","args":{}},"thoughtSignature":"c2tpcF90aG91Z2h0X3NpZ25hdHVyZV92YWxpZGF0b3I="}]"#,
}))]
#[case::vertex_prefixed_gemini_3_is_newer(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[{"function":{"name":"first","arguments":"{}"}}]}"#,
    model: Some("vertex_ai/gemini-3.5-flash"),
    forward: false,
    expected: r#"[{"function_call":{"name":"first","args":{}},"thoughtSignature":"c2tpcF90aG91Z2h0X3NpZ25hdHVyZV92YWxpZGF0b3I="}]"#,
}))]
#[case::unrecognized_names_count_as_newer(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[{"function":{"name":"first","arguments":"{}"}}]}"#,
    model: Some("test-id-bla"),
    forward: false,
    expected: r#"[{"function_call":{"name":"first","args":{}},"thoughtSignature":"c2tpcF90aG91Z2h0X3NpZ25hdHVyZV92YWxpZGF0b3I="}]"#,
}))]
#[case::numeric_fine_tuned_endpoint_is_older(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[{"function":{"name":"first","arguments":"{}"}}]}"#,
    model: Some("4965075652664360960"),
    forward: false,
    expected: r#"[{"function_call":{"name":"first","args":{}}}]"#,
}))]
#[case::gemma_model_is_older(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[{"function":{"name":"first","arguments":"{}"}}]}"#,
    model: Some("gemma-3-27b-it"),
    forward: false,
    expected: r#"[{"function_call":{"name":"first","args":{}}}]"#,
}))]
#[case::gemini_pro_is_older(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[{"function":{"name":"first","arguments":"{}"}}]}"#,
    model: Some("gemini-pro"),
    forward: false,
    expected: r#"[{"function_call":{"name":"first","args":{}}}]"#,
}))]
#[case::gemini_exp_is_older(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[{"function":{"name":"first","arguments":"{}"}}]}"#,
    model: Some("gemini-exp-1206"),
    forward: false,
    expected: r#"[{"function_call":{"name":"first","args":{}}}]"#,
}))]
#[case::empty_tool_calls_suppress_the_function_call(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[],"function_call":{"name":"ignored","arguments":"{}"}}"#,
    model: None,
    forward: false,
    expected: "[]",
}))]
#[case::null_tool_calls_fall_through_to_the_function_call(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":null,"function_call":{"name":"fallback","arguments":"{}"}}"#,
    model: None,
    forward: false,
    expected: r#"[{"function_call":{"name":"fallback","args":{}}}]"#,
}))]
#[case::entries_without_a_function_are_skipped(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[{"id":"call_custom","type":"custom","custom":{}},{"function":{"name":"real","arguments":""}}]}"#,
    model: None,
    forward: false,
    expected: r#"[{"function_call":{"name":"real","args":{"type":"object"}}}]"#,
}))]
#[case::forwarding_off_omits_ids(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[{"id":"call_a","function":{"name":"first","arguments":"{}"}}]}"#,
    model: None,
    forward: false,
    expected: r#"[{"function_call":{"name":"first","args":{}}}]"#,
}))]
#[case::missing_call_fields_yield_no_parts(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","content":"hi"}"#,
    model: None,
    forward: false,
    expected: "[]",
}))]
#[case::nested_arguments_survive_the_conversion(Scenario::Convert(ConvertCase {
    message: r#"{"role":"assistant","tool_calls":[{"function":{"name":"search","arguments":"{\"query\":\"x\",\"filters\":{\"year\":2024},\"tags\":[\"a\",\"b\"]}"}}]}"#,
    model: None,
    forward: false,
    expected: r#"[{"function_call":{"name":"search","args":{"query":"x","filters":{"year":2024},"tags":["a","b"]}}}]"#,
}))]
#[case::conversion_leaves_the_message_untouched(Scenario::Preserved(
    r#"{"role":"assistant","tool_calls":[{"id":"call_a__thought__sig","function":{"name":"first","arguments":"{\"a\":1}"}}]}"#,
))]
#[case::helper_strips_the_embedded_signature(Scenario::Helper(HelperCase {
    params: r#"{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}"#,
    tool_call_id: Some("call_abc__thought__sig"),
    expected: r#"{"name":"get_weather","args":{"city":"Paris"},"id":"call_abc"}"#,
}))]
#[case::helper_keeps_an_explicit_empty_id(Scenario::Helper(HelperCase {
    params: r#"{"name":"a","arguments":""}"#,
    tool_call_id: Some(""),
    expected: r#"{"name":"a","args":{"type":"object"},"id":""}"#,
}))]
#[case::helper_keeps_the_empty_head_of_a_signature_only_id(Scenario::Helper(HelperCase {
    params: r#"{"name":"a","arguments":"{}"}"#,
    tool_call_id: Some("__thought__sig"),
    expected: r#"{"name":"a","args":{},"id":""}"#,
}))]
#[case::helper_defaults_an_absent_name_to_empty(Scenario::Helper(HelperCase {
    params: r#"{"arguments":"{}"}"#,
    tool_call_id: None,
    expected: r#"{"name":"","args":{}}"#,
}))]
#[case::helper_null_arguments_use_the_schema_default(Scenario::Helper(HelperCase {
    params: r#"{"name":"a","arguments":null}"#,
    tool_call_id: None,
    expected: r#"{"name":"a","args":{"type":"object"}}"#,
}))]
#[case::helper_absent_arguments_use_the_schema_default(Scenario::Helper(HelperCase {
    params: r#"{"name":"a"}"#,
    tool_call_id: None,
    expected: r#"{"name":"a","args":{"type":"object"}}"#,
}))]
#[case::helper_keeps_non_object_arguments(Scenario::Helper(HelperCase {
    params: r#"{"name":"a","arguments":"[1,2]"}"#,
    tool_call_id: None,
    expected: r#"{"name":"a","args":[1,2]}"#,
}))]
#[case::helper_keeps_json_null_arguments(Scenario::Helper(HelperCase {
    params: r#"{"name":"a","arguments":"null"}"#,
    tool_call_id: None,
    expected: r#"{"name":"a","args":null}"#,
}))]
#[case::helper_rejects_malformed_arguments(Scenario::HelperFailure(HelperFailureCase {
    params: r#"{"name":"a","arguments":"{"}"#,
    tool_call_id: Some("call_a"),
    failure: Failure::Arguments,
}))]
#[case::encode_appends_the_separator_and_signature(Scenario::Encode(EncodeCase {
    tool_call_id: "call_abc",
    thought_signature: Some("sig"),
    expected: "call_abc__thought__sig",
}))]
#[case::encode_without_a_signature_returns_the_id(Scenario::Encode(EncodeCase {
    tool_call_id: "call_abc",
    thought_signature: None,
    expected: "call_abc",
}))]
#[case::encode_with_an_empty_signature_appends_the_separator(Scenario::Encode(EncodeCase {
    tool_call_id: "call_abc",
    thought_signature: Some(""),
    expected: "call_abc__thought__",
}))]
#[case::signature_prefers_tool_provider_fields(Scenario::Signature(SignatureCase {
    tool: r#"{"id":"call_x__thought__id-sig","provider_specific_fields":{"thought_signature":"tool-sig"},"function":{"name":"n","provider_specific_fields":{"thought_signature":"function-sig"}}}"#,
    expected: Some("tool-sig"),
}))]
#[case::signature_falls_back_to_function_provider_fields(Scenario::Signature(SignatureCase {
    tool: r#"{"id":"call_x__thought__id-sig","function":{"name":"n","provider_specific_fields":{"thought_signature":"function-sig"}}}"#,
    expected: Some("function-sig"),
}))]
#[case::signature_falls_back_to_the_embedded_id(Scenario::Signature(SignatureCase {
    tool: r#"{"id":"call_x__thought__id-sig","function":{"name":"n"}}"#,
    expected: Some("id-sig"),
}))]
#[case::signature_absent_is_none(Scenario::Signature(SignatureCase {
    tool: r#"{"id":"call_plain","function":{"name":"n"}}"#,
    expected: None,
}))]
#[case::signature_without_the_separator_is_none(Scenario::Signature(SignatureCase {
    tool: r#"{"id":"call_plain"}"#,
    expected: None,
}))]
#[case::signature_empty_provider_value_is_present(Scenario::Signature(SignatureCase {
    tool: r#"{"id":"call_x__thought__id-sig","provider_specific_fields":{"thought_signature":""}}"#,
    expected: Some(""),
}))]
#[case::signature_empty_id_part_is_present(Scenario::Signature(SignatureCase {
    tool: r#"{"id":"call_x__thought__"}"#,
    expected: Some(""),
}))]
#[case::protocol_null(Scenario::Protocol(ProtocolCase { value: "null", expected: ProtocolValue::Null }))]
#[case::protocol_number(Scenario::Protocol(ProtocolCase { value: "1.5", expected: ProtocolValue::Number }))]
#[case::protocol_text(Scenario::Protocol(ProtocolCase { value: "\"text\"", expected: ProtocolValue::String }))]
#[case::protocol_object(Scenario::Protocol(ProtocolCase { value: "{\"key\":\"value\"}", expected: ProtocolValue::Struct }))]
#[case::protocol_list(Scenario::Protocol(ProtocolCase { value: "[1,2]", expected: ProtocolValue::List }))]
#[case::protocol_bool(Scenario::Protocol(ProtocolCase { value: "true", expected: ProtocolValue::Bool }))]
#[case::dummy_signature_is_the_documented_base64(Scenario::Dummy)]
#[case::oversized_message_bytes_are_refused(Scenario::OversizedMessage)]
#[case::too_many_tool_calls_are_refused(Scenario::TooManyToolCalls)]
#[case::tool_calls_must_be_a_list(Scenario::ConvertFailure(ConvertFailureCase {
    message: r#"{"role":"assistant","tool_calls":{}}"#,
    model: None,
    forward: false,
    failure: Failure::Fields("tool_calls"),
}))]
#[case::function_must_be_an_object(Scenario::ConvertFailure(ConvertFailureCase {
    message: r#"{"role":"assistant","tool_calls":[{"function":"name"}]}"#,
    model: None,
    forward: false,
    failure: Failure::Fields("tool_calls[0].function"),
}))]
#[case::function_name_must_be_a_string(Scenario::ConvertFailure(ConvertFailureCase {
    message: r#"{"role":"assistant","tool_calls":[{"function":{"name":5}}]}"#,
    model: None,
    forward: false,
    failure: Failure::Fields("tool_calls[0].function.name"),
}))]
#[case::function_arguments_must_be_a_string(Scenario::ConvertFailure(ConvertFailureCase {
    message: r#"{"role":"assistant","tool_calls":[{"function":{"name":"a","arguments":5}}]}"#,
    model: None,
    forward: false,
    failure: Failure::Fields("tool_calls[0].function.arguments"),
}))]
#[case::thought_signature_must_be_a_string(Scenario::ConvertFailure(ConvertFailureCase {
    message: r#"{"role":"assistant","tool_calls":[{"function":{"name":"a","arguments":"{}","provider_specific_fields":{"thought_signature":5}}}]}"#,
    model: None,
    forward: false,
    failure: Failure::Fields(
        "tool_calls[0].function.provider_specific_fields.thought_signature",
    ),
}))]
#[case::provider_fields_must_be_an_object(Scenario::ConvertFailure(ConvertFailureCase {
    message: r#"{"role":"assistant","tool_calls":[{"provider_specific_fields":"x"}]}"#,
    model: None,
    forward: false,
    failure: Failure::Fields("tool_calls[0].provider_specific_fields"),
}))]
#[case::function_call_must_be_an_object(Scenario::ConvertFailure(ConvertFailureCase {
    message: r#"{"role":"assistant","function_call":5}"#,
    model: None,
    forward: false,
    failure: Failure::Fields("function_call"),
}))]
#[case::malformed_arguments_are_an_error(Scenario::ConvertFailure(ConvertFailureCase {
    message: r#"{"role":"assistant","tool_calls":[{"function":{"name":"a","arguments":"{"}}]}"#,
    model: Some("gemini-3-pro-preview"),
    forward: false,
    failure: Failure::Arguments,
}))]
#[case::invalid_message_bytes_are_an_error(Scenario::ParseFailure("{not json"))]
#[case::trailing_bytes_are_an_error(Scenario::ParseFailure("{} trailing"))]
fn gemini_tool_invokes_behavior_tests_behavior(#[case] scenario: Scenario) {
    run(scenario);
}

#[rstest]
fn replays_the_gemini_tool_invokes_corpus(
    #[files("fuzz/corpus/gemini_tool_invokes/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    if let Ok(message) = parse_tool_invoke_message(&bytes) {
        drop(convert_tool_calls_to_gemini(
            &message,
            Some("gemini-3-pro-preview"),
            true,
        ));
        drop(convert_tool_calls_to_gemini(&message, None, false));
    }
}
