#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::sdk::output::{FunctionCall, FunctionCallTag};
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use betterlite_openai::api::responses_shapes::{
    FunctionCallStatus, GenericResponseOutputItem, OutputFunctionToolCall,
};
use rstest::rstest;

use super::{
    McpToolCall, ToolSelectionError, extract_tool_call_details, extract_tool_calls_from_response,
    should_auto_execute_tools,
};
use crate::api::responses_mcp_events::function_call::function_call_id;

#[rstest]
#[case::single_never(r#"[{"require_approval":"never"}]"#, true)]
#[case::every_reference_never(
    r#"[{"require_approval":"never"},{"require_approval":"never"}]"#,
    true
)]
#[case::empty_list("[]", false)]
#[case::missing_key("[{}]", false)]
#[case::null_value(r#"[{"require_approval":null}]"#, false)]
#[case::always(r#"[{"require_approval":"always"}]"#, false)]
#[case::manual(r#"[{"require_approval":"manual"}]"#, false)]
#[case::object_form(r#"[{"require_approval":{"never":{"tools":["search"]}}}]"#, false)]
#[case::non_object_entry(r#"["never"]"#, false)]
#[case::one_always_beside_never(
    r#"[{"require_approval":"never"},{"require_approval":"always"}]"#,
    false
)]
#[case::non_string_value(r#"[{"require_approval":true}]"#, false)]
#[case::case_sensitive(r#"[{"require_approval":"Never"}]"#, false)]
fn auto_execute_requires_never(#[case] reference_list: &str, #[case] expected: bool) {
    let references: Vec<Json> =
        serde_json::from_str(reference_list).expect("the references decode");
    assert_eq!(should_auto_execute_tools(&references), expected);
}

#[rstest]
#[case::mixed_output(
    output_items(
        r#"[
            {"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[]},
            {"type":"function_call","call_id":"call_1","name":"weather","arguments":"{\"a\":1}"},
            {"type":"reasoning","id":"rs_1","summary":[]},
            {"type":"function_call","id":"fc_2","call_id":"call_2","name":"clock","arguments":"{}"}
        ]"#
    ),
    vec![Some("call_1"), Some("call_2")]
)]
#[case::sdk_arm(
    vec![item(ItemArm::PlainMessage), item(ItemArm::SdkFunctionCall)],
    vec![Some("call_sdk")]
)]
#[case::function_call_arm(
    vec![item(ItemArm::PlainMessage), item(ItemArm::TypedFunctionCall)],
    vec![Some("call_typed")]
)]
#[case::message_arm(
    vec![item(ItemArm::PlainMessage), item(ItemArm::FunctionCallMessage)],
    vec![Some("call_message")]
)]
#[case::unknown_arm(
    vec![item(ItemArm::PlainMessage), item(ItemArm::UnknownFunctionCall)],
    vec![Some("call_unknown")]
)]
#[case::function_call_arm_without_type(vec![item(ItemArm::UntypedFunctionCall)], vec![])]
#[case::message_arm_with_message_type(vec![item(ItemArm::PlainMessage)], vec![])]
#[case::unknown_without_type(vec![item(ItemArm::UnknownWithoutType)], vec![])]
#[case::unknown_with_other_type(vec![item(ItemArm::UnknownOtherType)], vec![])]
#[case::image_generation_arm(vec![item(ItemArm::ImageGeneration)], vec![])]
#[case::code_interpreter_arm(vec![item(ItemArm::CodeInterpreter)], vec![])]
#[case::custom_tool_arm(vec![item(ItemArm::CustomTool)], vec![])]
fn function_calls_in_output_order(
    #[case] output: Vec<ResponseOutputItem>,
    #[case] expected: Vec<Option<&str>>,
) {
    let response = response_with(output);
    let calls = extract_tool_calls_from_response(&response);
    assert_eq!(call_identifiers(&calls), expected);
}

#[derive(Clone, Copy)]
enum CallForm {
    Mapping,
    Output,
    Message,
    FunctionCall,
    Image,
}

#[rstest]
#[case::call_id_before_id(
    CallForm::Mapping,
    r#"{"call_id":"call_1","id":"fc_1","name":"weather"}"#,
    Some("weather"),
    None,
    Some("call_1")
)]
#[case::empty_call_id_falls_back_to_id(
    CallForm::Mapping,
    r#"{"call_id":"","id":"fc_1","name":"weather"}"#,
    Some("weather"),
    None,
    Some("fc_1")
)]
#[case::empty_call_id_and_empty_id_stay_empty(
    CallForm::Mapping,
    r#"{"call_id":"","id":""}"#,
    None,
    None,
    Some("")
)]
#[case::missing_call_id_uses_id(
    CallForm::Mapping,
    r#"{"id":"fc_1","name":"weather"}"#,
    Some("weather"),
    None,
    Some("fc_1")
)]
#[case::null_call_id_uses_id(
    CallForm::Mapping,
    r#"{"call_id":null,"id":"fc_1","name":"weather"}"#,
    Some("weather"),
    None,
    Some("fc_1")
)]
#[case::null_name_stays_empty(
    CallForm::Mapping,
    r#"{"name":null,"arguments":"{}"}"#,
    None,
    Some(r#""{}""#),
    None
)]
#[case::nested_function_wins_over_top_level(
    CallForm::Mapping,
    r#"{"call_id":"c","name":"outer","input":{"a":1},"function":{"name":"inner","arguments":"{\"b\":2}"}}"#,
    Some("inner"),
    Some(r#""{\"b\":2}""#),
    Some("c")
)]
#[case::nested_function_without_arguments_ignores_input(
    CallForm::Mapping,
    r#"{"name":"outer","input":{"a":1},"function":{"name":"inner"}}"#,
    Some("inner"),
    None,
    None
)]
#[case::arguments_before_input(
    CallForm::Mapping,
    r#"{"name":"weather","arguments":"{\"a\":1}","input":{"b":2}}"#,
    Some("weather"),
    Some(r#""{\"a\":1}""#),
    None
)]
#[case::empty_arguments_do_not_fall_back(
    CallForm::Mapping,
    r#"{"name":"weather","arguments":"","input":{"b":2}}"#,
    Some("weather"),
    Some(r#""""#),
    None
)]
#[case::input_used_when_arguments_null(
    CallForm::Mapping,
    r#"{"name":"weather","arguments":null,"input":{"b":2}}"#,
    Some("weather"),
    Some(r#"{"b":2}"#),
    None
)]
#[case::input_kept_as_object(
    CallForm::Mapping,
    r#"{"name":"weather","input":{"b":2}}"#,
    Some("weather"),
    Some(r#"{"b":2}"#),
    None
)]
#[case::text_argument_kept_as_text(
    CallForm::Mapping,
    r#"{"name":"weather","arguments":"plain"}"#,
    Some("weather"),
    Some(r#""plain""#),
    None
)]
#[case::no_arguments_or_input(
    CallForm::Mapping,
    r#"{"name":"weather"}"#,
    Some("weather"),
    None,
    None
)]
#[case::function_not_an_object_uses_top_level(
    CallForm::Mapping,
    r#"{"function":"x","name":"outer","arguments":"{}"}"#,
    Some("outer"),
    Some(r#""{}""#),
    None
)]
#[case::output_arm(
    CallForm::Output,
    r#"{"type":"function_call","call_id":"call_1","id":"fc_1","name":"weather","arguments":"{\"a\":1}"}"#,
    Some("weather"),
    Some(r#""{\"a\":1}""#),
    Some("call_1")
)]
#[case::output_arm_with_function_extra(
    CallForm::Output,
    r#"{"type":"function_call","call_id":"call_1","name":"top","arguments":"{}","function":{"name":"inner"}}"#,
    None,
    None,
    Some("call_1")
)]
#[case::output_arm_with_null_function_extra(
    CallForm::Output,
    r#"{"type":"function_call","call_id":"call_1","name":"top","arguments":"{}","function":null}"#,
    Some("top"),
    Some(r#""{}""#),
    Some("call_1")
)]
#[case::output_message_arm(
    CallForm::Output,
    r#"{"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[]}"#,
    None,
    None,
    None
)]
#[case::message_details(
    CallForm::Message,
    r#"{"type":"function_call","id":"msg_1","status":"completed","role":"assistant","content":[],"call_id":"call_message","name":"search","arguments":"{}"}"#,
    Some("search"),
    Some(r#""{}""#),
    Some("call_message")
)]
#[case::message_details_with_function_extra(
    CallForm::Message,
    r#"{"type":"function_call","id":"msg_2","status":"completed","role":"assistant","content":[],"call_id":"call_message","name":"search","arguments":"{}","function":{"name":"inner"}}"#,
    None,
    None,
    Some("call_message")
)]
#[case::message_details_input_when_arguments_null(
    CallForm::Message,
    r#"{"type":"function_call","id":"msg_3","status":"completed","role":"assistant","content":[],"call_id":"call_message","name":"search","arguments":null,"input":{"q":1}}"#,
    Some("search"),
    Some(r#"{"q":1}"#),
    Some("call_message")
)]
#[case::typed_function_call_arm(
    CallForm::FunctionCall,
    r#"{"type":"function_call","arguments":"{}","call_id":"call_typed","name":"clock","id":"fc_1","status":"completed"}"#,
    Some("clock"),
    Some(r#""{}""#),
    Some("call_typed")
)]
#[case::typed_function_call_input_when_arguments_null(
    CallForm::FunctionCall,
    r#"{"type":"function_call","arguments":null,"call_id":"call_typed","name":"clock","id":null,"input":{"q":1},"status":"completed"}"#,
    Some("clock"),
    Some(r#"{"q":1}"#),
    Some("call_typed")
)]
#[case::typed_function_call_with_function_extra(
    CallForm::FunctionCall,
    r#"{"type":"function_call","arguments":"{}","call_id":"call_typed","name":"clock","id":null,"status":"completed","function":{"name":"inner"}}"#,
    None,
    None,
    Some("call_typed")
)]
#[case::image_call_arm(
    CallForm::Image,
    r#"{"type":"image_generation_call","id":"img_1","status":"completed","result":null}"#,
    None,
    None,
    None
)]
fn tool_call_details_precedence(
    #[case] form: CallForm,
    #[case] tool_call: &str,
    #[case] name: Option<&str>,
    #[case] arguments: Option<&str>,
    #[case] call_id: Option<&str>,
) {
    let selected = match form {
        CallForm::Mapping => {
            McpToolCall::Fields(serde_json::from_str(tool_call).expect("the tool call decodes"))
        }
        CallForm::Output => McpToolCall::Output(Box::new(
            serde_json::from_str(tool_call).expect("the tool call decodes"),
        )),
        CallForm::Message => McpToolCall::Output(Box::new(ResponseOutputItem::Message(
            serde_json::from_str(tool_call).expect("the message decodes"),
        ))),
        CallForm::FunctionCall => McpToolCall::Output(Box::new(ResponseOutputItem::FunctionCall(
            serde_json::from_str(tool_call).expect("the typed call decodes"),
        ))),
        CallForm::Image => McpToolCall::Output(Box::new(ResponseOutputItem::ImageGeneration(
            serde_json::from_str(tool_call).expect("the image call decodes"),
        ))),
    };
    let details = extract_tool_call_details(&selected).expect("the details read");
    let expected_arguments = arguments
        .map(|text| serde_json::from_str::<Json>(text).expect("the expected arguments decode"));
    assert_eq!(details.name.as_deref(), name);
    assert_eq!(details.arguments, expected_arguments);
    assert_eq!(details.call_id.as_deref(), call_id);
}

#[rstest]
#[case::numeric_name(r#"{"name":5,"arguments":"{}"}"#, "name")]
#[case::numeric_call_id(r#"{"call_id":5,"id":"fc_1","name":"weather"}"#, "call_id")]
#[case::numeric_id(r#"{"id":5,"name":"weather"}"#, "id")]
#[case::numeric_nested_name(r#"{"function":{"name":5},"name":"outer"}"#, "name")]
#[case::numeric_id_after_empty_call_id(r#"{"call_id":"","id":5}"#, "id")]
fn tool_call_detail_errors(#[case] tool_call: &str, #[case] expected_field: &str) {
    let entries: BTreeMap<String, Json> =
        serde_json::from_str(tool_call).expect("the tool call decodes");
    let error = extract_tool_call_details(&McpToolCall::Fields(entries))
        .expect_err("a non-string field fails");
    let ToolSelectionError::NotAString { field } = error;
    assert_eq!(field, expected_field);
}

fn output_items(text: &str) -> Vec<ResponseOutputItem> {
    serde_json::from_str(text).expect("the output items decode")
}

fn response_with(output: Vec<ResponseOutputItem>) -> ResponsesApiResponse {
    let mut response: ResponsesApiResponse =
        serde_json::from_str(r#"{"id":"resp_1","created_at":1,"output":[]}"#)
            .expect("the response decodes");
    response.output = output;
    response
}

fn call_identifiers(calls: &[McpToolCall]) -> Vec<Option<&str>> {
    calls
        .iter()
        .map(|entry| match entry {
            McpToolCall::Output(item) => function_call_id(item),
            McpToolCall::Fields(_) => None,
        })
        .collect()
}

fn text_entries(pairs: &[(&str, &str)]) -> BTreeMap<String, Json> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), Json::Text((*value).to_owned())))
        .collect()
}

#[derive(Clone, Copy)]
enum ItemArm {
    SdkFunctionCall,
    TypedFunctionCall,
    UntypedFunctionCall,
    FunctionCallMessage,
    PlainMessage,
    UnknownFunctionCall,
    UnknownWithoutType,
    UnknownOtherType,
    ImageGeneration,
    CodeInterpreter,
    CustomTool,
}

const IMAGE_CALL: &str =
    r#"{"type":"image_generation_call","id":"img_1","status":"completed","result":null}"#;

const CODE_INTERPRETER_CALL: &str = r#"{"type":"code_interpreter_call","id":"ci_1","code":null,"container_id":null,"status":"completed","outputs":null}"#;

const CUSTOM_TOOL_CALL: &str = r#"{"type":"custom_tool_call","call_id":"ct_1","id":"ct_1","name":"f","input":"","status":null}"#;

fn sdk_call() -> ResponseOutputItem {
    ResponseOutputItem::Sdk(Box::new(SdkOutput::FunctionCall(Box::new(FunctionCall {
        r#type: FunctionCallTag::Value,
        arguments: r#"{"city":"Paris"}"#.to_owned(),
        call_id: "call_sdk".to_owned(),
        name: "weather".to_owned(),
        id: Some("fc_sdk".to_owned()),
        caller: None,
        namespace: None,
        status: None,
        extra: BTreeMap::new(),
    }))))
}

fn typed_call(item_type: Option<&str>, arguments: Option<&str>) -> ResponseOutputItem {
    ResponseOutputItem::FunctionCall(OutputFunctionToolCall {
        arguments: arguments.map(str::to_owned),
        call_id: Some("call_typed".to_owned()),
        name: Some("clock".to_owned()),
        r#type: item_type.map(str::to_owned),
        id: None,
        status: FunctionCallStatus::Completed,
        phase: None,
        extra: BTreeMap::new(),
    })
}

fn message_call(item_type: &str, id: &str, extra: &[(&str, &str)]) -> ResponseOutputItem {
    ResponseOutputItem::Message(GenericResponseOutputItem {
        r#type: item_type.to_owned(),
        id: id.to_owned(),
        status: "completed".to_owned(),
        role: "assistant".to_owned(),
        content: Vec::new(),
        phase: None,
        extra: text_entries(extra),
    })
}

fn item(arm: ItemArm) -> ResponseOutputItem {
    match arm {
        ItemArm::SdkFunctionCall => sdk_call(),
        ItemArm::TypedFunctionCall => typed_call(Some("function_call"), Some("{}")),
        ItemArm::UntypedFunctionCall => typed_call(None, None),
        ItemArm::FunctionCallMessage => message_call(
            "function_call",
            "msg_fc",
            &[
                ("call_id", "call_message"),
                ("name", "search"),
                ("arguments", "{}"),
            ],
        ),
        ItemArm::PlainMessage => message_call("message", "msg_1", &[]),
        ItemArm::UnknownFunctionCall => ResponseOutputItem::Unknown(text_entries(&[
            ("type", "function_call"),
            ("call_id", "call_unknown"),
            ("name", "lookup"),
            ("arguments", "{}"),
        ])),
        ItemArm::UnknownWithoutType => {
            ResponseOutputItem::Unknown(text_entries(&[("name", "lookup")]))
        }
        ItemArm::UnknownOtherType => {
            ResponseOutputItem::Unknown(text_entries(&[("type", "message"), ("id", "x_1")]))
        }
        ItemArm::ImageGeneration => ResponseOutputItem::ImageGeneration(
            serde_json::from_str(IMAGE_CALL).expect("the image call decodes"),
        ),
        ItemArm::CodeInterpreter => ResponseOutputItem::CodeInterpreter(
            serde_json::from_str(CODE_INTERPRETER_CALL).expect("the code interpreter call decodes"),
        ),
        ItemArm::CustomTool => ResponseOutputItem::CustomTool(
            serde_json::from_str(CUSTOM_TOOL_CALL).expect("the custom tool call decodes"),
        ),
    }
}
