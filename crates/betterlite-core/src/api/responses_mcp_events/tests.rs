#![cfg(test)]

use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use betterlite_openai::api::responses_shapes::{GenericResponseOutputItem, OutputFunctionToolCall};
use rstest::rstest;
use serde_json::{Value, json};

use super::call::{McpCallRequest, create_mcp_call_events};
use super::error::McpEventError;
use super::event::{McpEvent, set_event_field as assign_field};
use super::function_call::function_call_id;
use super::list_tools::{
    DiagnosticLevel, DiscoveredTool, ServerEntry, ToolAttribute, create_mcp_list_tools_events,
};

fn call_request(request: Value) -> McpCallRequest {
    serde_json::from_value(request).expect("call request")
}

fn call_events(request: Value, ids: &[&str]) -> Vec<McpEvent> {
    let mut remaining = ids.iter();
    create_mcp_call_events(&call_request(request), || {
        (*remaining.next().expect("unused id requested")).to_owned()
    })
    .expect("call events")
}

fn servers(entries: Value) -> Vec<ServerEntry> {
    serde_json::from_value(entries).expect("server entries")
}

fn named_tool(name: &str) -> DiscoveredTool {
    DiscoveredTool {
        name: ToolAttribute::Value(name.to_owned()),
        description: ToolAttribute::Absent,
        input_schema: ToolAttribute::Absent,
    }
}

fn to_json(events: &[McpEvent]) -> Value {
    serde_json::to_value(events).expect("events serialize")
}

#[rstest]
#[case::falsy_call_id_falls_back_to_id(
    json!([{"type": "function_call", "call_id": "", "id": "fallback-7"}]),
    vec![Some("fallback-7")]
)]
#[case::call_id_wins_over_id(
    json!([{"type": "function_call", "call_id": "call-1", "id": "other"}]),
    vec![Some("call-1")]
)]
#[case::message_is_not_a_function_call(
    json!([{"type": "message", "id": "ignored"}]),
    vec![None]
)]
#[case::numeric_id_is_not_a_string(
    json!([{"type": "function_call", "id": 9}]),
    vec![None]
)]
#[case::missing_both_ids(json!([{"type": "function_call"}]), vec![None])]
#[case::empty_call_id_and_empty_id_stay_empty(
    json!([{"type": "function_call", "call_id": "", "id": ""}]),
    vec![Some("")]
)]
#[case::typed_function_call_object(
    json!([{
        "type": "function_call", "call_id": "object-8", "arguments": "{}",
        "name": "weather", "id": "fc_1", "status": "completed"
    }]),
    vec![Some("object-8")]
)]
#[case::order_is_retained_across_kinds(
    json!([
        {"type": "message", "id": "ignored"},
        {"type": "function_call", "call_id": "", "id": "fallback-7"},
        {"type": "function_call", "id": 9},
        {"type": "function_call", "call_id": "z"}
    ]),
    vec![None, Some("fallback-7"), None, Some("z")]
)]
fn output_items_and_function_call_id(#[case] output: Value, #[case] expected: Vec<Option<&str>>) {
    let response: ResponsesApiResponse = serde_json::from_value(json!({
        "id": "resp_1", "created_at": 1, "output": output
    }))
    .expect("response");
    let identifiers: Vec<Option<&str>> = response.output.iter().map(function_call_id).collect();
    assert_eq!(identifiers, expected);
}

#[rstest]
#[case::object_function_call(
    json!({"type": "function_call", "call_id": "object-8", "name": "weather",
           "arguments": "{}", "id": "fc_8", "status": "completed"}),
    Some("object-8")
)]
#[case::object_other_type(
    json!({"type": "message", "call_id": "object-8", "name": "weather",
           "arguments": "{}", "id": "fc_8", "status": "completed"}),
    None
)]
fn output_items_and_function_call_id_reads_an_object_item(
    #[case] fields: Value,
    #[case] expected: Option<&str>,
) {
    let call: OutputFunctionToolCall = serde_json::from_value(fields).expect("call");
    let item = ResponseOutputItem::FunctionCall(call);
    assert_eq!(function_call_id(&item), expected);
}

#[rstest]
#[case::message_without_call_id(json!({}), Some("msg_1"))]
#[case::message_with_call_id(json!({"call_id": "c1"}), Some("c1"))]
fn output_items_and_function_call_id_reads_a_message_shaped_call(
    #[case] extra: Value,
    #[case] expected: Option<&str>,
) {
    let mut fields = json!({"type": "function_call", "id": "msg_1", "status": "completed",
                            "role": "assistant", "content": []});
    fields
        .as_object_mut()
        .expect("fields")
        .extend(extra.as_object().expect("extra").clone());
    let message: GenericResponseOutputItem = serde_json::from_value(fields).expect("message");
    let item = ResponseOutputItem::Message(message);
    assert_eq!(function_call_id(&item), expected);
}

fn sample_call_event() -> McpEvent {
    call_events(
        json!({"tool_name": "w", "arguments": "{}", "base_item_id": "mcp_x", "output_index": 1}),
        &[],
    )
    .remove(0)
}

#[rstest]
#[case::known_integer_field("output_index", json!(4), "/output_index", json!(4))]
#[case::sequence_number("sequence_number", json!(9), "/sequence_number", json!(9))]
#[case::new_field_is_serialized("custom_marker", json!("kept"), "/custom_marker", json!("kept"))]
#[case::object_valued_new_field("flag", json!({"a": [1]}), "/flag/a/0", json!(1))]
#[case::item_id("item_id", json!("mcp_other"), "/item_id", json!("mcp_other"))]
#[case::delta_on_event_without_one("delta", json!("{}"), "/delta", json!("{}"))]
#[case::type_change(
    "type",
    json!("response.mcp_call.completed"),
    "/type",
    json!("response.mcp_call.completed")
)]
#[case::item(
    "item",
    json!({"type": "mcp_list_tools", "id": "i", "server_label": "s", "tools": []}),
    "/item/server_label",
    json!("s")
)]
fn set_event_field(
    #[case] name: &str,
    #[case] value: Value,
    #[case] pointer: &str,
    #[case] expected: Value,
) {
    let mut event = sample_call_event();
    assert_eq!(event.output_index, 1);
    assign_field(&mut event, name, value).expect("field set");
    let serialized = serde_json::to_value(&event).expect("serialize");
    assert_eq!(serialized.pointer(pointer), Some(&expected));
}

#[rstest]
#[case::integer_expected("output_index", json!("4"))]
#[case::string_expected("item_id", json!(4))]
fn set_event_field_rejects_wrong_types(#[case] name: &str, #[case] value: Value) {
    let mut event = sample_call_event();
    let before = event.clone();
    let error = assign_field(&mut event, name, value).expect_err("wrong type");
    assert!(matches!(error, McpEventError::FieldType { field, .. } if field == name));
    assert_eq!(event, before);
}

#[test]
fn set_event_field_rejects_an_unknown_event_type() {
    let mut event = sample_call_event();
    let before = event.clone();
    let error =
        assign_field(&mut event, "type", json!("response.unknown")).expect_err("event type");
    assert!(matches!(error, McpEventError::EventType { source } if source.is_data()));
    assert_eq!(event, before);
}

#[test]
fn set_event_field_rejects_a_malformed_item() {
    let mut event = sample_call_event();
    let error = assign_field(&mut event, "item", json!({"type": "other"})).expect_err("item");
    assert!(matches!(error, McpEventError::ItemShape { .. }));
}

#[rstest]
#[case::mapped_label_and_two_tools(
    json!([
        {"server_url": "litellm_proxy/mcp/weather", "server_label": "weather"},
        {"server_url": "https://host.example/mcp/other", "server_label": "ignored"}
    ]),
    "weather"
)]
#[case::no_servers(json!([]), "")]
#[case::missing_label(json!([{"server_url": "litellm_proxy/mcp/x"}]), "")]
#[case::null_label(json!([{"server_label": null}]), "")]
#[case::only_the_first_mapping_counts(json!([{}, {"server_label": "later"}]), "")]
fn list_tools_success(#[case] entries: Value, #[case] label: &str) {
    let tools = [
        DiscoveredTool {
            name: ToolAttribute::Value("search".to_owned()),
            description: ToolAttribute::Value("find".to_owned()),
            input_schema: ToolAttribute::Value(json!({"type": "object"})),
        },
        DiscoveredTool {
            name: ToolAttribute::Absent,
            description: ToolAttribute::Absent,
            input_schema: ToolAttribute::Absent,
        },
    ];
    let outcome = create_mcp_list_tools_events(&servers(entries), "mcp_base01", &tools);
    assert_eq!(
        to_json(&outcome.events),
        json!([
            {"type": "response.mcp_list_tools.in_progress", "sequence_number": 1,
             "output_index": 0, "item_id": "mcp_base01"},
            {"type": "response.mcp_list_tools.completed", "sequence_number": 2,
             "output_index": 0, "item_id": "mcp_base01"},
            {"type": "response.output_item.done", "sequence_number": 1, "output_index": 0,
             "item": {"type": "mcp_list_tools", "id": "mcp_base01", "server_label": label,
                "tools": [
                    {"name": "search", "description": "find",
                     "annotations": {"read_only": false}, "input_schema": {"type": "object"}},
                    {"name": "unknown", "description": "",
                     "annotations": {"read_only": false}}
                ]}}
        ])
    );
    assert_eq!(outcome.diagnostics.len(), 1);
    let diagnostic = outcome.diagnostics.first().expect("diagnostic");
    assert_eq!(diagnostic.level, DiagnosticLevel::Debug);
    assert_eq!(diagnostic.message, "Created 3 MCP discovery events");
}

#[rstest]
#[case::name_raises(DiscoveredTool {
    name: ToolAttribute::Raises("boom".to_owned()),
    description: ToolAttribute::Absent,
    input_schema: ToolAttribute::Absent,
})]
#[case::description_raises(DiscoveredTool {
    name: ToolAttribute::Absent,
    description: ToolAttribute::Raises("boom".to_owned()),
    input_schema: ToolAttribute::Absent,
})]
#[case::input_schema_raises(DiscoveredTool {
    name: ToolAttribute::Absent,
    description: ToolAttribute::Absent,
    input_schema: ToolAttribute::Raises("boom".to_owned()),
})]
fn list_tools_failure(#[case] failing: DiscoveredTool) {
    let tools = [named_tool("ok"), failing];
    let entries = servers(json!([{"server_label": "kept-out"}]));
    let outcome = create_mcp_list_tools_events(&entries, "mcp_bad01", &tools);
    assert_eq!(
        to_json(&outcome.events),
        json!([
            {"type": "response.mcp_list_tools.in_progress", "sequence_number": 1,
             "output_index": 0, "item_id": "mcp_bad01"},
            {"type": "response.mcp_list_tools.completed", "sequence_number": 2,
             "output_index": 0, "item_id": "mcp_bad01"},
            {"type": "response.mcp_list_tools.failed", "sequence_number": 2,
             "output_index": 0, "item_id": "mcp_bad01"},
            {"type": "response.output_item.done", "sequence_number": 1, "output_index": 0,
             "item": {"type": "mcp_list_tools", "id": "mcp_bad01", "server_label": "",
                "tools": []}}
        ])
    );
    let reported: Vec<(DiagnosticLevel, &str)> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| (diagnostic.level, diagnostic.message.as_str()))
        .collect();
    assert_eq!(
        reported,
        [
            (
                DiagnosticLevel::Error,
                "Error creating MCP list tools events: boom"
            ),
            (DiagnosticLevel::Traceback, "boom"),
        ]
    );
}

#[rstest]
#[case::offset_four_index_three(4, 3, [4, 5, 6, 7])]
#[case::defaults_one_and_zero(1, 0, [1, 2, 3, 4])]
fn mcp_call_success(
    #[case] sequence_start: i64,
    #[case] output_index: i64,
    #[case] sequences: [i64; 4],
) {
    let events = call_events(
        json!({
            "tool_name": "weather", "tool_call_id": "upstream-call-7",
            "arguments": "{\"city\":\"Berlin\"}", "result": "{\"temp\":7}",
            "base_item_id": "mcp_call01", "sequence_start": sequence_start,
            "output_index": output_index
        }),
        &["22222222"],
    );
    let [first, second, third, fourth] = sequences;
    assert_eq!(
        to_json(&events),
        json!([
            {"type": "response.mcp_call.in_progress", "sequence_number": first,
             "output_index": output_index, "item_id": "mcp_call01"},
            {"type": "response.mcp_call_arguments.delta", "sequence_number": second,
             "output_index": output_index, "item_id": "mcp_call01",
             "delta": "{\"city\":\"Berlin\"}"},
            {"type": "response.mcp_call_arguments.done", "sequence_number": third,
             "output_index": output_index, "item_id": "mcp_call01",
             "arguments": "{\"city\":\"Berlin\"}"},
            {"type": "response.mcp_call.completed", "sequence_number": fourth,
             "output_index": output_index, "item_id": "mcp_call01"},
            {"type": "response.output_item.done", "sequence_number": 1,
             "output_index": output_index,
             "item": {"type": "mcp_call", "id": "mcp_call01",
                "approval_request_id": "mcpr_22222222", "arguments": "{\"city\":\"Berlin\"}",
                "error": null, "name": "weather", "output": "{\"temp\":7}",
                "server_label": "litellm"}}
        ])
    );
}

#[test]
fn mcp_call_failure() {
    let events = call_events(
        json!({
            "tool_name": "weather", "tool_call_id": "upstream-call-7", "arguments": "{}",
            "result": null, "base_item_id": "mcp_call02", "sequence_start": 9,
            "output_index": 5
        }),
        &[],
    );
    assert_eq!(
        to_json(&events),
        json!([
            {"type": "response.mcp_call.in_progress", "sequence_number": 9,
             "output_index": 5, "item_id": "mcp_call02"},
            {"type": "response.mcp_call_arguments.delta", "sequence_number": 10,
             "output_index": 5, "item_id": "mcp_call02", "delta": "{}"},
            {"type": "response.mcp_call_arguments.done", "sequence_number": 11,
             "output_index": 5, "item_id": "mcp_call02", "arguments": "{}"},
            {"type": "response.mcp_call.failed", "sequence_number": 12,
             "output_index": 5, "item_id": "mcp_call02"}
        ])
    );
}

#[test]
fn mcp_call_defaults_and_offsets() {
    let failed = call_events(
        json!({"tool_name": "", "tool_call_id": null, "arguments": ""}),
        &["11111111", "22222222"],
    );
    assert_eq!(failed.len(), 4);
    let last = failed.last().expect("failed event");
    assert_eq!(
        serde_json::to_value(last).expect("serialize"),
        json!({"type": "response.mcp_call.failed", "sequence_number": 4,
               "output_index": 0, "item_id": "mcp_11111111"})
    );

    let completed = call_events(
        json!({"tool_name": "", "arguments": "", "result": ""}),
        &["11111111", "22222222"],
    );
    assert_eq!(completed.len(), 5);
    let done = serde_json::to_value(completed.last().expect("done event")).expect("serialize");
    assert_eq!(done.pointer("/item/id"), Some(&json!("mcp_11111111")));
    assert_eq!(
        done.pointer("/item/approval_request_id"),
        Some(&json!("mcpr_22222222"))
    );
    assert_eq!(done.pointer("/item/output"), Some(&json!("")));
    assert_eq!(done.pointer("/item/server_label"), Some(&json!("litellm")));
    assert_eq!(done.pointer("/sequence_number"), Some(&json!(1)));
}

#[test]
fn mcp_call_defaults_and_offsets_treats_an_empty_base_id_as_unset() {
    let events = call_events(
        json!({"tool_name": "t", "arguments": "{}", "base_item_id": ""}),
        &["abcdef01"],
    );
    let first = serde_json::to_value(events.first().expect("event")).expect("serialize");
    assert_eq!(first.pointer("/item_id"), Some(&json!("mcp_abcdef01")));
}

#[test]
fn mcp_call_defaults_and_offsets_reports_sequence_overflow() {
    let request = call_request(json!({
        "tool_name": "t", "arguments": "{}", "base_item_id": "mcp_x",
        "sequence_start": i64::MAX
    }));
    let error = create_mcp_call_events(&request, String::new).expect_err("overflow");
    assert!(matches!(error, McpEventError::SequenceOverflow { .. }));
}
