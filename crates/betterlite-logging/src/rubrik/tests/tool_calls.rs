use betterlite_logging::rubrik::tool_calls::{
    ChatCompletionMessageToolCall, Function, ToolCallError, ToolCallSource, normalize_tool_call,
    normalize_tool_calls,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

fn call(id: &str, name: &str, arguments: &str) -> ChatCompletionMessageToolCall {
    ChatCompletionMessageToolCall {
        id: id.to_owned(),
        kind: "function".to_owned(),
        function: Function {
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        },
    }
}

#[rstest]
#[case::already_normalized(ToolCallSource::Normalized(call("call_1", "get_weather", "{}")))]
fn passes_through_an_already_normalized_tool_call(#[case] source: ToolCallSource) {
    let normalized = normalize_tool_call(&source).expect("normalizes");
    assert_eq!(normalized, call("call_1", "get_weather", "{}"));
}

fn raw_tool_call_dict(id: &str, kind: &str, name: &str, arguments: &str) -> Json {
    Json::Object(IndexMap::from([
        ("id".to_owned(), Json::Text(id.to_owned())),
        ("type".to_owned(), Json::Text(kind.to_owned())),
        (
            "function".to_owned(),
            Json::Object(IndexMap::from([
                ("name".to_owned(), Json::Text(name.to_owned())),
                ("arguments".to_owned(), Json::Text(arguments.to_owned())),
            ])),
        ),
    ]))
}

#[rstest]
#[case::streaming_chunk_shape(
    raw_tool_call_dict("call_2", "function", "get_weather", "{\"city\":\"NY\"}"),
    call("call_2", "get_weather", "{\"city\":\"NY\"}")
)]
#[case::generic_dict_shape(
    raw_tool_call_dict("call_3", "function", "get_time", "{}"),
    call("call_3", "get_time", "{}")
)]
fn normalizes_a_dict_shaped_tool_call(
    #[case] value: Json,
    #[case] expected: ChatCompletionMessageToolCall,
) {
    let source = ToolCallSource::Raw(value);
    let normalized = normalize_tool_call(&source).expect("normalizes");
    assert_eq!(normalized, expected);
}

#[test]
fn normalizes_a_protocol_object_exposing_id_and_function_attributes() {
    let source = ToolCallSource::Protocol {
        id: Some("call_4".to_owned()),
        kind: None,
        function: Function {
            name: "lookup".to_owned(),
            arguments: "{}".to_owned(),
        },
    };
    let normalized = normalize_tool_call(&source).expect("normalizes");
    assert_eq!(normalized, call("call_4", "lookup", "{}"));
}

#[test]
fn defaults_missing_fields_on_a_dict_shaped_tool_call() {
    let value = Json::Object(IndexMap::new());
    let source = ToolCallSource::Raw(value);
    let normalized = normalize_tool_call(&source).expect("normalizes");
    assert_eq!(normalized, call("", "", ""));
}

#[test]
fn rejects_an_unrecognized_tool_call_shape() {
    let source = ToolCallSource::Raw(Json::Text("not a tool call".to_owned()));
    let result = normalize_tool_call(&source);
    assert_eq!(result, Err(ToolCallError::UnsupportedShape));
}

#[test]
fn normalizes_a_sequence_of_tool_calls_in_order() {
    let sources = vec![
        ToolCallSource::Normalized(call("call_a", "a", "{}")),
        ToolCallSource::Raw(raw_tool_call_dict("call_b", "function", "b", "{}")),
    ];
    let normalized = normalize_tool_calls(&sources).expect("normalizes");
    assert_eq!(normalized[0].id, "call_a");
    assert_eq!(normalized[1].id, "call_b");
}
