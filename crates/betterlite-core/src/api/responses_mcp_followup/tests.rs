#![cfg(test)]
#![expect(
    clippy::too_many_lines,
    reason = "a behavior test asserts one whole parameter plan, event stream, output pair or provider call"
)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use betterlite_openai::response_input::ResponsesInput;
use rstest::rstest;
use serde::de::DeserializeOwned;

use super::error::{McpFollowupError, ToolExecutionEventError};
use super::events::{EventToolResult, LiteLlmCallEvents, ToolCallId, create_tool_execution_events};
use super::output::add_mcp_output_elements_to_response;
use super::records::{
    FollowUpCall, IdSource, MetadataAuth, ResponsesMcpFollowupInput, StreamingAuth,
};
use super::streaming::{StreamingError, StreamingRequest, create_mcp_streaming_response};
use super::{FollowUpProvider, ResponsesMcpFollowup};
use crate::api::responses_mcp_events::event::{EventField, EventItem, McpEventKind, StreamEvent};
use crate::api::responses_mcp_results::McpToolResult;
use crate::api::responses_mcp_tool_selection::McpToolCall;
use crate::api::responses_provider_selection::ParameterMap;

fn parse<T>(text: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(text).unwrap()
}

fn params(entries: &[(&str, &str)]) -> ParameterMap {
    entries
        .iter()
        .map(|(name, value)| (String::from(*name), parse::<JsonValue>(value)))
        .collect()
}

fn ids(values: &[&str]) -> IdSource {
    let remaining = Arc::new(Mutex::new(VecDeque::from_iter(
        values.iter().map(|value| String::from(*value)),
    )));
    Arc::new(move || remaining.lock().unwrap().pop_front().unwrap_or_default())
}

fn auth(user_id: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = parse(&format!(r#"{{"user_id": "{user_id}"}}"#));
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

#[derive(serde::Deserialize)]
struct AuthView {
    user_id: Option<String>,
}

fn user_id(auth: &Option<UserAPIKeyAuth>) -> Option<String> {
    let principal = auth.as_ref()?;
    let text = serde_json::to_string(principal).unwrap();
    parse::<AuthView>(&text).user_id
}

fn no_auth() -> StreamingAuth {
    StreamingAuth {
        direct: None,
        metadata: MetadataAuth::Absent,
    }
}

fn tool_call(text: &str) -> McpToolCall {
    McpToolCall::Fields(parse(text))
}

fn discovery_event() -> StreamEvent {
    StreamEvent::new(McpEventKind::CallInProgress, 1, 0)
}

fn sample_input() -> ResponsesMcpFollowupInput {
    ResponsesMcpFollowupInput {
        input: ResponsesInput::Text(String::from("what is the weather")),
        model: String::from("gpt-5"),
        tools: Some(parse(r#"[{"type":"function","name":"lookup"}]"#)),
        previous_response_id: Some(String::from("resp_previous")),
        call_params: params(&[("stream", "true"), ("temperature", "0.2")]),
        kwargs: params(&[("top_p", "0.9")]),
        follow_up_input: vec![parse(
            r#"{"type":"function_call_output","call_id":"call_1","output":"sunny"}"#,
        )],
        follow_up_response_id: Some(String::from("resp_previous")),
        mcp_tools_with_litellm_proxy: vec![parse(r#"{"type":"mcp","server_label":"weather"}"#)],
        mcp_discovery_events: vec![discovery_event()],
        tool_server_map: BTreeMap::from([(
            String::from("weather"),
            String::from("http://weather.example"),
        )]),
        auth: no_auth(),
        tool_calls: vec![tool_call(
            r#"{"call_id":"call_1","name":"get_weather","arguments":"{\"city\":\"Paris\"}"}"#,
        )],
        tool_results: vec![McpToolResult {
            tool_call_id: Some(String::from("call_1")),
            result: String::from("sunny"),
            name: None,
        }],
        mcp_tools_fetched: vec![parse(r#"{"type":"mcp","server_label":"weather"}"#)],
    }
}

fn streaming_request(input: &ResponsesMcpFollowupInput) -> StreamingRequest<'_> {
    StreamingRequest {
        input: &input.input,
        model: &input.model,
        tools: input.tools.as_deref(),
        previous_response_id: input.previous_response_id.as_deref(),
        call_params: &input.call_params,
        kwargs: &input.kwargs,
        mcp_tools_with_litellm_proxy: &input.mcp_tools_with_litellm_proxy,
        mcp_discovery_events: &input.mcp_discovery_events,
        tool_server_map: &input.tool_server_map,
        auth: &input.auth,
    }
}

fn key_names(params: &ParameterMap) -> Vec<&str> {
    params.keys().map(String::as_str).collect()
}

#[test]
fn request_param_precedence() {
    let mut input = sample_input();
    input.call_params = params(&[
        ("temperature", "0.2"),
        ("model", "\"gpt-5-override\""),
        ("stream", "null"),
    ]);
    input.kwargs = params(&[
        ("top_p", "0.9"),
        ("tools", r#"[{"type":"mcp","server_label":"weather"}]"#),
    ]);
    input.auth = StreamingAuth {
        direct: Some(auth("alice")),
        metadata: MetadataAuth::NotMapping,
    };
    let plan = create_mcp_streaming_response(&streaming_request(&input)).unwrap();
    assert_eq!(
        key_names(&plan.request_params),
        vec![
            "input",
            "model",
            "tools",
            "previous_response_id",
            "temperature",
            "stream",
            "top_p",
        ]
    );
    assert_eq!(
        plan.request_params["input"],
        JsonValue::Text(String::from("what is the weather"))
    );
    assert_eq!(
        plan.request_params["model"],
        JsonValue::Text(String::from("gpt-5-override"))
    );
    assert_eq!(
        plan.request_params["tools"],
        parse::<JsonValue>(r#"[{"type":"mcp","server_label":"weather"}]"#)
    );
    assert_eq!(
        plan.request_params["previous_response_id"],
        JsonValue::Text(String::from("resp_previous"))
    );
    assert_eq!(plan.request_params["stream"], JsonValue::Null);
    assert_eq!(plan.request_params["top_p"], parse::<JsonValue>("0.9"));
    assert_eq!(user_id(&plan.auth), Some(String::from("alice")));
    assert_eq!(plan.mcp_events, input.mcp_discovery_events);
    assert_eq!(plan.tool_server_map, input.tool_server_map);
    assert_eq!(
        plan.mcp_tools_with_litellm_proxy,
        input.mcp_tools_with_litellm_proxy
    );

    let mut absent = sample_input();
    absent.previous_response_id = None;
    absent.tools = None;
    absent.call_params = ParameterMap::new();
    absent.kwargs = ParameterMap::new();
    let absent_plan = create_mcp_streaming_response(&streaming_request(&absent)).unwrap();
    assert_eq!(
        key_names(&absent_plan.request_params),
        vec!["input", "model", "tools"]
    );
    assert_eq!(absent_plan.request_params["tools"], JsonValue::Null);
    assert_eq!(user_id(&absent_plan.auth), None);

    let mut metadata = sample_input();
    metadata.auth = StreamingAuth {
        direct: None,
        metadata: MetadataAuth::Mapping(Some(auth("bob"))),
    };
    let metadata_plan = create_mcp_streaming_response(&streaming_request(&metadata)).unwrap();
    assert_eq!(user_id(&metadata_plan.auth), Some(String::from("bob")));

    let mut broken = sample_input();
    broken.auth = StreamingAuth {
        direct: None,
        metadata: MetadataAuth::NotMapping,
    };
    assert_eq!(
        create_mcp_streaming_response(&streaming_request(&broken)).err(),
        Some(StreamingError::MetadataNotMapping)
    );
}

fn text_field(event: &StreamEvent, name: &str) -> String {
    match event.fields.get(name) {
        Some(EventField::Json(value)) => String::from(value.as_str().unwrap_or_default()),
        Some(EventField::Item(_)) | None => String::new(),
    }
}

fn number_field(event: &StreamEvent, name: &str) -> i64 {
    match event.fields.get(name) {
        Some(EventField::Json(value)) => value.as_i64().unwrap_or_default(),
        Some(EventField::Item(_)) | None => 0,
    }
}

struct CallItem<'event> {
    approval_request_id: &'event str,
    arguments: &'event str,
    name: &'event str,
    output: &'event str,
}

fn call_item(event: &StreamEvent) -> CallItem<'_> {
    let Some(EventField::Item(EventItem::Call {
        approval_request_id,
        arguments,
        name,
        output,
        ..
    })) = event.fields.get("item")
    else {
        panic!("the output_item.done event carries an mcp_call item");
    };
    CallItem {
        approval_request_id: approval_request_id.as_str(),
        arguments: arguments.as_str(),
        name: name.as_str(),
        output: output.as_str(),
    }
}

fn event_results(entries: &[(&str, &str)]) -> Vec<EventToolResult> {
    entries
        .iter()
        .map(|(tool_call_id, result)| match *tool_call_id {
            "absent" => EventToolResult {
                tool_call_id: ToolCallId::Absent,
                result: String::from(*result),
            },
            "null" => EventToolResult {
                tool_call_id: ToolCallId::Null,
                result: String::from(*result),
            },
            found => EventToolResult {
                tool_call_id: ToolCallId::Value(String::from(found)),
                result: String::from(*result),
            },
        })
        .collect()
}

#[test]
fn tool_execution_event_order() {
    let tool_calls = vec![
        tool_call(
            r#"{"call_id":"call_1","name":"get_weather","arguments":"{\"city\":\"Paris\"}"}"#,
        ),
        tool_call(r#"{"call_id":"call_2","name":"","arguments":""}"#),
    ];
    let results = event_results(&[("call_1", "sunny"), ("call_2", ""), ("missing", "x")]);
    let source = ids(&[
        "a1b2c3d4e5f6",
        "b2c3d4e5f6a7",
        "c3d4e5f6a7b8",
        "d4e5f6a7b8c9",
        "e5f6a7b8c9d0",
        "f6a7b8c9d0e1",
    ]);
    let events =
        create_tool_execution_events(&LiteLlmCallEvents, &tool_calls, &results, &source).unwrap();
    let kinds: Vec<String> = events
        .iter()
        .map(|event| text_field(event, "type"))
        .collect();
    assert_eq!(
        kinds,
        vec![
            "response.mcp_call.in_progress",
            "response.mcp_call_arguments.delta",
            "response.mcp_call_arguments.done",
            "response.mcp_call.completed",
            "response.output_item.done",
            "response.mcp_call.in_progress",
            "response.mcp_call_arguments.delta",
            "response.mcp_call_arguments.done",
            "response.mcp_call.completed",
            "response.output_item.done",
            "response.mcp_call.in_progress",
            "response.mcp_call_arguments.delta",
            "response.mcp_call_arguments.done",
            "response.mcp_call.completed",
            "response.output_item.done",
        ]
    );
    let sequences: Vec<i64> = events
        .iter()
        .map(|event| number_field(event, "sequence_number"))
        .collect();
    assert_eq!(
        sequences,
        vec![1, 2, 3, 4, 1, 6, 7, 8, 9, 1, 11, 12, 13, 14, 1]
    );
    let item_ids: Vec<String> = events
        .iter()
        .filter_map(|event| match event.fields.get("item_id") {
            Some(EventField::Json(value)) => value.as_str().map(String::from),
            Some(EventField::Item(_)) | None => None,
        })
        .collect();
    assert_eq!(
        item_ids,
        vec![
            "mcp_a1b2c3d4",
            "mcp_a1b2c3d4",
            "mcp_a1b2c3d4",
            "mcp_a1b2c3d4",
            "mcp_c3d4e5f6",
            "mcp_c3d4e5f6",
            "mcp_c3d4e5f6",
            "mcp_c3d4e5f6",
            "mcp_e5f6a7b8",
            "mcp_e5f6a7b8",
            "mcp_e5f6a7b8",
            "mcp_e5f6a7b8",
        ]
    );
    let outputs: Vec<String> = events
        .iter()
        .filter(|event| text_field(event, "type") == "response.output_item.done")
        .map(|event| String::from(call_item(event).output))
        .collect();
    assert_eq!(outputs, vec!["sunny", "", "x"]);
    let first = call_item(&events[4]);
    assert_eq!(first.name, "get_weather");
    assert_eq!(first.arguments, r#"{"city":"Paris"}"#);
    assert_eq!(first.approval_request_id, "mcpr_b2c3d4e5");
    let second = call_item(&events[9]);
    assert_eq!(second.name, "unknown");
    assert_eq!(second.arguments, "{}");
    assert_eq!(second.approval_request_id, "mcpr_d4e5f6a7");
    let third = call_item(&events[14]);
    assert_eq!(third.name, "unknown");
    assert_eq!(third.arguments, "{}");
    assert_eq!(third.approval_request_id, "mcpr_f6a7b8c9");
    assert_eq!(
        number_field(&events[9], "output_index"),
        0,
        "every event reports output_index 0"
    );
}

#[rstest]
#[case::name_missing(None, None, "unknown", "{}")]
#[case::name_empty(Some(""), Some(""), "unknown", "{}")]
#[case::name_present(
    Some("get_weather"),
    Some(r#"{"city":"Paris"}"#),
    "get_weather",
    r#"{"city":"Paris"}"#
)]
fn falsy_names_and_arguments_take_defaults(
    #[case] name: Option<&str>,
    #[case] arguments: Option<&str>,
    #[case] expected_name: &str,
    #[case] expected_arguments: &str,
) {
    let mut fields =
        BTreeMap::from([(String::from("call_id"), Json::Text(String::from("call_1")))]);
    if let Some(text) = name {
        fields.insert(String::from("name"), Json::Text(String::from(text)));
    }
    if let Some(text) = arguments {
        fields.insert(String::from("arguments"), Json::Text(String::from(text)));
    }
    let results = event_results(&[("call_1", "sunny")]);
    let events = create_tool_execution_events(
        &LiteLlmCallEvents,
        &[McpToolCall::Fields(fields)],
        &results,
        &ids(&["a1b2c3d4e5f6", "b2c3d4e5f6a7"]),
    )
    .unwrap();
    let item = call_item(&events[4]);
    assert_eq!(item.name, expected_name);
    assert_eq!(item.arguments, expected_arguments);
}

#[test]
fn matching_uses_the_first_call_and_the_missing_or_null_identity() {
    let first = tool_call(r#"{"call_id":"call_1","name":"first","arguments":"{}"}"#);
    let second = tool_call(r#"{"call_id":"call_1","name":"second","arguments":"{}"}"#);
    let results = event_results(&[("call_1", "one"), ("absent", "two"), ("null", "three")]);
    let events = create_tool_execution_events(
        &LiteLlmCallEvents,
        &[first, second],
        &results,
        &ids(&[
            "a1b2c3d4e5f6",
            "b2c3d4e5f6a7",
            "c3d4e5f6a7b8",
            "d4e5f6a7b8c9",
            "e5f6a7b8c9d0",
            "f6a7b8c9d0e1",
        ]),
    )
    .unwrap();
    assert_eq!(call_item(&events[4]).name, "first");
    assert_eq!(call_item(&events[9]).name, "unknown");

    let named_unknown = tool_call(r#"{"call_id":"unknown","name":"implicit","arguments":"{}"}"#);
    let null_call = tool_call(r#"{"name":"without_id","arguments":"{}"}"#);
    let second_results = event_results(&[("absent", "two"), ("null", "three")]);
    let second_events = create_tool_execution_events(
        &LiteLlmCallEvents,
        &[named_unknown, null_call],
        &second_results,
        &ids(&[
            "a1b2c3d4e5f6",
            "b2c3d4e5f6a7",
            "c3d4e5f6a7b8",
            "d4e5f6a7b8c9",
        ]),
    )
    .unwrap();
    assert_eq!(call_item(&second_events[4]).name, "implicit");
    assert_eq!(call_item(&second_events[9]).name, "without_id");
}

#[test]
fn unreadable_tool_call_fields_are_reported() {
    let results = event_results(&[("call_1", "sunny")]);
    let bad_name = tool_call(r#"{"call_id":"call_1","name":7,"arguments":"{}"}"#);
    let error = create_tool_execution_events(
        &LiteLlmCallEvents,
        &[bad_name],
        &results,
        &ids(&["a1b2c3d4e5f6", "b2c3d4e5f6a7"]),
    )
    .err()
    .unwrap();
    assert!(matches!(error, ToolExecutionEventError::Details(_)));
    let object_arguments =
        tool_call(r#"{"call_id":"call_1","name":"get_weather","arguments":{"city":"Paris"}}"#);
    let arguments_error = create_tool_execution_events(
        &LiteLlmCallEvents,
        &[object_arguments],
        &results,
        &ids(&["a1b2c3d4e5f6", "b2c3d4e5f6a7"]),
    )
    .err()
    .unwrap();
    assert!(matches!(
        arguments_error,
        ToolExecutionEventError::ArgumentsNotString
    ));
}

#[derive(Clone, Copy)]
struct Scenario {
    tool_choice: Option<&'static str>,
    original_stream: bool,
    should_auto_execute: bool,
}

#[rstest]
#[case::absent(Scenario { tool_choice: None, original_stream: true, should_auto_execute: true })]
#[case::required(Scenario { tool_choice: Some("\"required\""), original_stream: true, should_auto_execute: false })]
#[case::auto(Scenario { tool_choice: Some("\"auto\""), original_stream: false, should_auto_execute: true })]
#[case::named(Scenario { tool_choice: Some(r#"{"type":"function","name":"get_weather"}"#), original_stream: false, should_auto_execute: false })]
#[case::null(Scenario { tool_choice: Some("null"), original_stream: false, should_auto_execute: true })]
fn call_parameter_streaming(#[case] scenario: Scenario) {
    let mut input = sample_input();
    input.call_params = params(&[("stream", "true"), ("temperature", "0.2")]);
    if let Some(tool_choice) = scenario.tool_choice {
        input
            .call_params
            .insert(String::from("tool_choice"), parse(tool_choice));
    }
    let provider = FakeProvider::answering(
        r#"{"id":"resp_final","created_at":2,"status":"completed","output":[]}"#,
    );
    let builder = LiteLlmCallEvents;
    let service = ResponsesMcpFollowup {
        provider: &provider,
        ids: ids(&["a1b2c3d4e5f6", "b2c3d4e5f6a7"]),
        event_builder: &builder,
        should_auto_execute: scenario.should_auto_execute,
        original_stream: scenario.original_stream,
    };
    let result = futures_executor::block_on(service.run(&input)).unwrap();
    assert_eq!(
        result.initial_params["temperature"],
        parse::<JsonValue>("0.2")
    );
    assert_eq!(
        result.initial_params["stream"],
        JsonValue::Bool(!scenario.should_auto_execute)
    );
    assert_eq!(
        result.follow_up_params["stream"],
        JsonValue::Bool(scenario.original_stream)
    );
    assert_eq!(
        key_names(&result.follow_up_params),
        vec!["stream", "temperature"]
    );
    assert_eq!(result.call.kwargs, result.follow_up_params);
    if scenario.original_stream {
        assert!(result.streaming.is_some());
        assert!(result.response.is_none());
        assert!(provider.calls().is_empty());
    } else {
        assert!(result.streaming.is_none());
        assert_eq!(provider.calls(), vec![result.call.clone()]);
    }
}

#[test]
fn output_element_append_order() {
    let mut response: ResponsesApiResponse = parse(
        r#"{"id":"resp_1","created_at":1,"output":[{"type":"previous_item","id":"prev_1","payload":{"kept":true}}]}"#,
    );
    let existing = response.output.clone();
    let fetched: Vec<Json> = vec![
        parse(r#"{"type":"mcp","server_label":"weather"}"#),
        parse(r#"{"type":"mcp","server_label":"time"}"#),
    ];
    let results = vec![
        McpToolResult {
            tool_call_id: Some(String::from("call_1")),
            result: String::from("sunny and warm, café"),
            name: Some(String::from("get_weather")),
        },
        McpToolResult {
            tool_call_id: None,
            result: String::new(),
            name: None,
        },
    ];
    add_mcp_output_elements_to_response(
        &mut response,
        &fetched,
        &results,
        &ids(&[
            "0123456789abcdef0123456789abcdef",
            "fedcba9876543210fedcba9876543210",
        ]),
    );
    assert_eq!(&response.output[..1], existing.as_slice());
    assert_eq!(response.output.len(), 3);
    let ResponseOutputItem::Message(tools_item) = &response.output[1] else {
        panic!("the first appended item is a message");
    };
    assert_eq!(tools_item.r#type, "mcp_tools_fetched");
    assert_eq!(tools_item.id, "mcp_tools_01234567");
    assert_eq!(tools_item.status, "completed");
    assert_eq!(tools_item.role, "system");
    assert_eq!(tools_item.phase, None);
    assert_eq!(tools_item.content.len(), 1);
    assert_eq!(tools_item.content[0].r#type.as_deref(), Some("output_text"));
    assert_eq!(tools_item.content[0].annotations, Some(Vec::new()));
    assert_eq!(
        tools_item.content[0].text.as_deref(),
        Some(
            r#"[
  {
    "type": "mcp",
    "server_label": "weather"
  },
  {
    "type": "mcp",
    "server_label": "time"
  }
]"#
        )
    );
    let ResponseOutputItem::Message(results_item) = &response.output[2] else {
        panic!("the second appended item is a message");
    };
    assert_eq!(results_item.r#type, "tool_execution_results");
    assert_eq!(results_item.id, "tool_results_fedcba98");
    assert_eq!(results_item.status, "completed");
    assert_eq!(results_item.role, "system");
    assert_eq!(results_item.phase, None);
    assert_eq!(
        results_item.content[0].text.as_deref(),
        Some(
            r#"[
  {
    "tool_call_id": "call_1",
    "result": "sunny and warm, caf\u00e9",
    "name": "get_weather"
  },
  {
    "tool_call_id": null,
    "result": "",
    "name": null
  }
]"#
        )
    );
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("provider failure")]
struct ProviderFailure {
    status: u16,
}

struct FakeProvider {
    answers: Mutex<VecDeque<Result<ResponsesApiResponse, ProviderFailure>>>,
    calls: Mutex<Vec<FollowUpCall>>,
}

impl FakeProvider {
    fn answering(response: &str) -> Self {
        Self {
            answers: Mutex::new(VecDeque::from([Ok(parse(response))])),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing(status: u16) -> Self {
        Self {
            answers: Mutex::new(VecDeque::from([Err(ProviderFailure { status })])),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<FollowUpCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl FollowUpProvider for FakeProvider {
    type Error = ProviderFailure;

    async fn call(&self, call: &FollowUpCall) -> Result<ResponsesApiResponse, ProviderFailure> {
        self.calls.lock().unwrap().push(call.clone());
        self.answers.lock().unwrap().pop_front().unwrap()
    }
}

#[tokio::test]
async fn provider_call_error() {
    let mut input = sample_input();
    input.call_params = params(&[
        ("stream", "true"),
        ("tool_choice", "\"required\""),
        ("temperature", "0.2"),
    ]);
    let provider = FakeProvider::failing(429);
    let builder = LiteLlmCallEvents;
    let service = ResponsesMcpFollowup {
        provider: &provider,
        ids: ids(&["a1b2c3d4e5f6", "b2c3d4e5f6a7"]),
        event_builder: &builder,
        should_auto_execute: true,
        original_stream: false,
    };
    let error = service.run(&input).await.err().unwrap();
    let calls = provider.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(
        call.input,
        ResponsesInput::Items(input.follow_up_input.clone())
    );
    assert_eq!(call.model, "gpt-5");
    assert_eq!(call.tools, input.tools);
    assert_eq!(
        call.previous_response_id,
        Some(String::from("resp_previous"))
    );
    assert_eq!(
        call.kwargs,
        params(&[("stream", "false"), ("temperature", "0.2")])
    );
    let McpFollowupError::Provider(failure) = error else {
        panic!("the provider error reaches the caller unchanged");
    };
    assert_eq!(failure.status, 429);
}

#[tokio::test]
async fn completed_response_appends_output_elements_in_place() {
    let mut input = sample_input();
    input.previous_response_id = None;
    input.tools = None;
    let provider = FakeProvider::answering(
        r#"{"id":"resp_final","created_at":2,"status":"completed","output":[{"type":"previous_item","id":"prev_final"}]}"#,
    );
    let builder = LiteLlmCallEvents;
    let service = ResponsesMcpFollowup {
        provider: &provider,
        ids: ids(&[
            "0123456789abcdef0123456789abcdef",
            "fedcba9876543210fedcba9876543210",
        ]),
        event_builder: &builder,
        should_auto_execute: true,
        original_stream: false,
    };
    let result = service.run(&input).await.unwrap();
    assert_eq!(provider.calls(), vec![result.call.clone()]);
    assert_eq!(result.call.previous_response_id, None);
    assert_eq!(result.call.tools, None);
    assert!(result.streaming.is_none());
    assert!(result.events.is_empty());
    let response = result.response.unwrap();
    assert_eq!(response.output.len(), 3);
    let ResponseOutputItem::Message(appended) = &response.output[2] else {
        panic!("the appended tool results item is a message");
    };
    assert_eq!(appended.id, "tool_results_fedcba98");
    assert_eq!(response.status.as_deref(), Some("completed"));
}
