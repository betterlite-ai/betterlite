#![cfg(test)]

use std::collections::BTreeMap;

use super::identity;
use super::mcp;
use super::passthrough;
use super::set_attributes;
use super::truthy::is_truthy;
use crate::otel::integrations::arize_messages::ArizeOtelAttributes;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span as SdkSpan};

struct Recorded {
    attributes: BTreeMap<String, Value>,
    events: Vec<(String, BTreeMap<String, Value>)>,
}

fn map(text: &str) -> MetadataMap {
    serde_json::from_str(text).expect("map")
}

fn value(text: &str) -> OtelModelValue {
    serde_json::from_str(text).expect("value")
}

fn attr<'source>(attributes: &'source BTreeMap<String, Value>, key: &str) -> Option<&'source str> {
    match attributes.get(key) {
        Some(Value::String(text)) => Some(text.as_str()),
        Some(_) | None => None,
    }
}

fn record<F>(record: F) -> Recorded
where
    F: FnOnce(&mut SdkSpan),
{
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("arize-attributes").start("test");
    record(&mut span);
    span.end();
    let spans = exporter.get_finished_spans().expect("spans");
    let finished = spans.first().expect("one span");
    Recorded {
        attributes: finished
            .attributes
            .iter()
            .map(|pair| (pair.key.to_string(), pair.value.clone()))
            .collect(),
        events: finished
            .events
            .events
            .iter()
            .map(|event| {
                (
                    event.name.to_string(),
                    event
                        .attributes
                        .iter()
                        .map(|pair| (pair.key.to_string(), pair.value.clone()))
                        .collect(),
                )
            })
            .collect(),
    }
}

fn flow(kwargs: &MetadataMap, response: &OtelModelValue, redact_messages: bool) -> Recorded {
    record(|span| {
        set_attributes::<ArizeOtelAttributes, SdkSpan>(span, kwargs, response, redact_messages);
    })
}

fn attributes_of(kwargs: &str, response: &str) -> BTreeMap<String, Value> {
    flow(&map(kwargs), &value(response), false).attributes
}

fn standard_logging_object(kwargs: &MetadataMap) -> Option<MetadataMap> {
    kwargs
        .get("standard_logging_object")
        .and_then(OtelModelValue::as_object)
        .cloned()
}

fn with_payload(payload: &str) -> String {
    format!(r#"{{"standard_logging_object":{payload}}}"#)
}

#[test]
fn attributes_chat_request_writes_request_attributes() {
    let attributes = attributes_of(
        r#"{"model":"gpt-4o","optional_params":{"max_tokens":100,"temperature":0.5,"top_p":0.9,"stream":true,"user":"user-1"},"litellm_params":{"custom_llm_provider":"openai"},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(attr(&attributes, "llm.model_name"), Some("gpt-4o"));
    assert_eq!(attributes.get("llm.provider"), Some(&Value::from("openai")));
    assert_eq!(attr(&attributes, "llm.request.type"), Some("acompletion"));
    assert_eq!(
        attributes.get("llm.request.max_tokens"),
        Some(&Value::I64(100))
    );
    assert_eq!(
        attributes.get("llm.request.temperature"),
        Some(&Value::F64(0.5))
    );
    assert_eq!(attributes.get("llm.request.top_p"), Some(&Value::F64(0.9)));
    assert_eq!(attributes.get("llm.user"), Some(&Value::from("user-1")));
    assert_eq!(attr(&attributes, "openinference.span.kind"), Some("LLM"));
}

#[test]
fn attributes_sampling_params_at_zero_are_skipped() {
    let attributes = attributes_of(
        r#"{"optional_params":{"max_tokens":0,"temperature":0,"top_p":0},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert!(!attributes.contains_key("llm.request.max_tokens"));
    assert!(!attributes.contains_key("llm.request.temperature"));
    assert!(!attributes.contains_key("llm.request.top_p"));
}

#[test]
fn attributes_falsy_model_user_and_response_identity_are_skipped() {
    let attributes = attributes_of(
        r#"{"model":"","optional_params":{"user":""},"standard_logging_object":{"call_type":"acompletion"}}"#,
        r#"{"id":"","model":""}"#,
    );
    assert!(!attributes.contains_key("llm.model_name"));
    assert!(!attributes.contains_key("llm.user"));
    assert!(!attributes.contains_key("llm.response.id"));
    assert!(!attributes.contains_key("llm.response.model"));
}

#[rstest::rstest]
#[case::true_value(r#"{"stream":true}"#, "True")]
#[case::false_value(r#"{"stream":false}"#, "False")]
#[case::null_value(r#"{"stream":null}"#, "None")]
#[case::absent("{}", "False")]
fn attributes_streaming_flag_is_python_bool_text(
    #[case] optional_params: &str,
    #[case] text: &str,
) {
    let kwargs = format!(
        r#"{{"optional_params":{optional_params},"standard_logging_object":{{"call_type":"acompletion"}}}}"#
    );
    let attributes = attributes_of(&kwargs, "null");
    assert_eq!(attr(&attributes, "llm.is_streaming"), Some(text));
}

#[test]
fn attributes_provider_defaults_to_unknown_when_absent() {
    let attributes = attributes_of(&with_payload(r#"{"call_type":"acompletion"}"#), "null");
    assert_eq!(attr(&attributes, "llm.provider"), Some("Unknown"));
}

#[test]
fn attributes_null_provider_is_written_as_empty_string() {
    let attributes = attributes_of(
        r#"{"litellm_params":{"custom_llm_provider":null},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(attributes.get("llm.provider"), Some(&Value::from("")));
}

#[test]
fn attributes_request_type_is_empty_when_call_type_is_absent() {
    let attributes = attributes_of(&with_payload("{}"), "null");
    assert_eq!(attributes.get("llm.request.type"), Some(&Value::from("")));
    assert_eq!(
        attr(&attributes, "openinference.span.kind"),
        Some("UNKNOWN")
    );
}

#[rstest::rstest]
#[case::embedding(r#"{"call_type":"aembedding"}"#, "EMBEDDING")]
#[case::completion(r#"{"call_type":"acompletion"}"#, "LLM")]
#[case::mcp_tool(r#"{"call_type":"call_mcp_tool"}"#, "TOOL")]
#[case::missing_call_type("{}", "UNKNOWN")]
fn attributes_span_kind_follows_call_type(#[case] payload: &str, #[case] kind: &str) {
    let attributes = attributes_of(&with_payload(payload), "null");
    assert_eq!(attr(&attributes, "openinference.span.kind"), Some(kind));
}

#[test]
fn attributes_metadata_is_written_as_json_text() {
    let attributes = attributes_of(
        &with_payload(r#"{"call_type":"acompletion","metadata":{"trace_name":"demo"}}"#),
        "null",
    );
    assert_eq!(
        attr(&attributes, "metadata"),
        Some(r#"{"trace_name":"demo"}"#)
    );
}

#[test]
fn attributes_null_metadata_is_skipped() {
    let attributes = attributes_of(
        &with_payload(r#"{"call_type":"acompletion","metadata":null}"#),
        "null",
    );
    assert!(!attributes.contains_key("metadata"));
}

#[test]
fn attributes_messages_and_response_use_the_shared_writers() {
    let attributes = attributes_of(
        r#"{"messages":[{"role":"system","content":"be brief"},{"role":"user","content":"hello"}],"standard_logging_object":{"call_type":"acompletion"}}"#,
        r#"{"id":"chatcmpl-1","model":"gpt-4o","choices":[{"message":{"role":"assistant","content":"hello there"}}]}"#,
    );
    assert_eq!(attributes.get("input.value"), Some(&Value::from("hello")));
    assert_eq!(
        attr(&attributes, "llm.input_messages.0.message.role"),
        Some("system")
    );
    assert_eq!(
        attr(&attributes, "llm.input_messages.1.message.content"),
        Some("hello")
    );
    assert_eq!(attr(&attributes, "llm.response.id"), Some("chatcmpl-1"));
    assert_eq!(attr(&attributes, "llm.response.model"), Some("gpt-4o"));
    assert_eq!(attr(&attributes, "output.value"), Some("hello there"));
    assert_eq!(
        attr(&attributes, "llm.output_messages.0.message.role"),
        Some("assistant")
    );
    assert_eq!(
        attr(&attributes, "llm.output_messages.0.message.content"),
        Some("hello there")
    );
}

#[test]
fn attributes_tools_from_optional_params_and_metadata() {
    let attributes = attributes_of(
        r#"{"optional_params":{"tools":[{"type":"function","function":{"name":"get_weather","description":"Weather","parameters":{"type":"object"}}}]},"standard_logging_object":{"call_type":"acompletion","metadata":{"llm":{"tools":[{"name":"meta_tool","description":"Meta"}]}}}}"#,
        "null",
    );
    assert_eq!(attr(&attributes, "llm.tools.0.name"), Some("get_weather"));
    assert_eq!(
        attr(&attributes, "llm.tools.0.description"),
        Some("Weather")
    );
    assert_eq!(
        attr(&attributes, "llm.tools.0.parameters"),
        Some(r#"{"type":"object"}"#)
    );
    assert_eq!(
        attr(&attributes, "llm.invocation_parameters.tools.0.name"),
        Some("meta_tool")
    );
    assert_eq!(
        attr(&attributes, "llm.invocation_parameters.tools.0.description"),
        Some("Meta")
    );
}

#[test]
fn attributes_model_parameters_json_and_user_are_written() {
    let attributes = attributes_of(
        &with_payload(
            r#"{"call_type":"acompletion","model_parameters":{"temperature":0.2,"user":"model-user"}}"#,
        ),
        "null",
    );
    assert_eq!(
        attr(&attributes, "llm.invocation_parameters"),
        Some(r#"{"temperature":0.2,"user":"model-user"}"#)
    );
    assert_eq!(attributes.get("user.id"), Some(&Value::from("model-user")));
}

#[test]
fn attributes_empty_model_parameters_are_skipped() {
    let attributes = attributes_of(
        &with_payload(r#"{"call_type":"acompletion","model_parameters":{}}"#),
        "null",
    );
    assert!(!attributes.contains_key("llm.invocation_parameters"));
    assert!(!attributes.contains_key("user.id"));
}

#[test]
fn attributes_missing_payload_writes_span_kind_and_exception_event() {
    let recorded = flow(
        &map(r#"{"model":"gpt-4o","optional_params":{"stream":true}}"#),
        &value("null"),
        false,
    );
    assert_eq!(
        recorded.attributes,
        BTreeMap::from([("openinference.span.kind".to_owned(), Value::from("UNKNOWN"))])
    );
    assert_eq!(recorded.events.len(), 1);
    let (name, event_attributes) = &recorded.events[0];
    assert_eq!(name, "exception");
    assert_eq!(
        event_attributes.get("exception.type"),
        Some(&Value::from("ValueError"))
    );
    assert_eq!(
        event_attributes.get("exception.message"),
        Some(&Value::from("standard_logging_object not found in kwargs"))
    );
}

#[rstest::rstest]
#[case::null("null", false)]
#[case::false_value("false", false)]
#[case::true_value("true", true)]
#[case::zero("0", false)]
#[case::non_zero("1", true)]
#[case::empty_string(r#""""#, false)]
#[case::text(r#""a""#, true)]
#[case::empty_array("[]", false)]
#[case::array("[1]", true)]
#[case::empty_object("{}", false)]
#[case::object(r#"{"a":1}"#, true)]
fn attributes_is_truthy_matches_litellm_truthiness(#[case] raw: &str, #[case] expected: bool) {
    assert_eq!(is_truthy(&value(raw)), expected);
}

fn session_attributes(kwargs: &str) -> BTreeMap<String, Value> {
    let source = map(kwargs);
    let payload = standard_logging_object(&source);
    record(|span| identity::set_session_and_user_attrs(span, &source, payload.as_ref())).attributes
}

fn cost_attributes(payload: &str) -> BTreeMap<String, Value> {
    let source = map(&with_payload(payload));
    let standard_logging_object = standard_logging_object(&source);
    record(|span| identity::set_response_cost_attr(span, standard_logging_object.as_ref()))
        .attributes
}

#[test]
fn session_end_user_team_and_key_alias_are_written() {
    let attributes = session_attributes(
        r#"{"optional_params":{"user":"request-user"},"standard_logging_object":{"call_type":"acompletion","trace_id":"trace-1","metadata":{"user_api_key_end_user_id":"end-user-1","user_api_key_user_id":"key-user-1","user_api_key_team_id":"team-1","user_api_key_team_alias":"Team One","user_api_key_alias":"key-alias-1"}}}"#,
    );
    assert_eq!(attr(&attributes, "session.id"), Some("end-user-1"));
    assert_eq!(attr(&attributes, "litellm.trace_id"), Some("trace-1"));
    assert_eq!(attributes.get("user.id"), None);
    assert_eq!(attr(&attributes, "litellm.team_id"), Some("team-1"));
    assert_eq!(attr(&attributes, "litellm.team_alias"), Some("Team One"));
    assert_eq!(attr(&attributes, "litellm.key_alias"), Some("key-alias-1"));
}

#[test]
fn session_user_id_from_key_metadata_when_no_request_user() {
    let attributes = session_attributes(
        r#"{"standard_logging_object":{"call_type":"acompletion","metadata":{"user_api_key_user_id":"key-user-1"}}}"#,
    );
    assert_eq!(attributes.get("user.id"), Some(&Value::from("key-user-1")));
}

#[test]
fn session_user_id_skipped_when_optional_user_set() {
    let attributes = session_attributes(
        r#"{"optional_params":{"user":"request-user"},"standard_logging_object":{"call_type":"acompletion","metadata":{"user_api_key_user_id":"key-user-1"}}}"#,
    );
    assert_eq!(attributes.get("user.id"), None);
}

#[test]
fn session_user_id_skipped_when_model_parameters_user_set() {
    let attributes = session_attributes(
        r#"{"standard_logging_object":{"call_type":"acompletion","model_parameters":{"user":"model-user"},"metadata":{"user_api_key_user_id":"key-user-1"}}}"#,
    );
    assert_eq!(attributes.get("user.id"), None);
}

#[test]
fn session_empty_values_are_skipped() {
    let attributes = session_attributes(
        r#"{"standard_logging_object":{"call_type":"acompletion","trace_id":"","metadata":{"user_api_key_end_user_id":"","user_api_key_user_id":"","user_api_key_team_id":"","user_api_key_team_alias":"","user_api_key_alias":""}}}"#,
    );
    assert!(attributes.is_empty());
}

#[test]
fn session_skipped_without_standard_logging_object() {
    let attributes = session_attributes(
        r#"{"optional_params":{"user":"request-user"},"metadata":{"user_api_key_user_id":"key-user-1"}}"#,
    );
    assert!(attributes.is_empty());
}

#[test]
fn session_metadata_not_an_object_is_skipped() {
    let attributes = session_attributes(
        r#"{"standard_logging_object":{"call_type":"acompletion","trace_id":"trace-1","metadata":"plain"}}"#,
    );
    assert!(attributes.is_empty());
}

#[rstest::rstest]
#[case::null(r#"{"standard_logging_object":{"call_type":"acompletion","trace_id":"trace-1","metadata":null}}"#)]
#[case::empty_string(
    r#"{"standard_logging_object":{"call_type":"acompletion","trace_id":"trace-1","metadata":""}}"#
)]
fn session_missing_metadata_still_writes_trace_id(#[case] kwargs: &str) {
    let attributes = session_attributes(kwargs);
    assert_eq!(attr(&attributes, "litellm.trace_id"), Some("trace-1"));
}

#[test]
fn cost_number_is_written_as_a_float_to_both_keys() {
    let attributes = cost_attributes(r#"{"call_type":"acompletion","response_cost":1}"#);
    assert_eq!(attributes.get("llm.cost.total"), Some(&Value::F64(1.0)));
    assert_eq!(attributes.get("llm.response.cost"), Some(&Value::F64(1.0)));
}

#[test]
fn cost_string_is_parsed_after_trimming() {
    let attributes = cost_attributes(r#"{"call_type":"acompletion","response_cost":" 0.25 "}"#);
    assert_eq!(attributes.get("llm.cost.total"), Some(&Value::F64(0.25)));
    assert_eq!(attributes.get("llm.response.cost"), Some(&Value::F64(0.25)));
}

#[rstest::rstest]
#[case::true_value("true", 1.0_f64)]
#[case::false_value("false", 0.0_f64)]
fn cost_bool_becomes_one_or_zero(#[case] raw: &str, #[case] expected: f64) {
    let attributes = cost_attributes(&format!(
        r#"{{"call_type":"acompletion","response_cost":{raw}}}"#
    ));
    assert_eq!(
        attributes.get("llm.cost.total"),
        Some(&Value::F64(expected))
    );
    assert_eq!(
        attributes.get("llm.response.cost"),
        Some(&Value::F64(expected))
    );
}

#[rstest::rstest]
#[case::invalid_string(r#""not-a-number""#)]
#[case::nan(r#""nan""#)]
#[case::infinity(r#""inf""#)]
#[case::array("[]")]
#[case::object("{}")]
fn cost_invalid_values_write_nothing(#[case] raw: &str) {
    let attributes = cost_attributes(&format!(
        r#"{{"call_type":"acompletion","response_cost":{raw}}}"#
    ));
    assert!(attributes.is_empty());
}

#[test]
fn cost_null_and_absent_write_nothing() {
    assert!(cost_attributes(r#"{"call_type":"acompletion","response_cost":null}"#).is_empty());
    assert!(cost_attributes(r#"{"call_type":"acompletion"}"#).is_empty());
    assert!(
        record(|span| identity::set_response_cost_attr(span, None))
            .attributes
            .is_empty()
    );
}

fn flow_attributes<F>(kwargs: &str, response: &str, apply: F) -> BTreeMap<String, Value>
where
    F: FnOnce(&mut SdkSpan, &MetadataMap, &OtelModelValue, Option<&MetadataMap>),
{
    let source = map(kwargs);
    let response_value = value(response);
    let payload = standard_logging_object(&source);
    record(|span| apply(span, &source, &response_value, payload.as_ref())).attributes
}

fn passthrough_attributes(
    kwargs: &str,
    response: &str,
    redact_messages: bool,
) -> BTreeMap<String, Value> {
    flow_attributes(kwargs, response, |span, source, response_value, payload| {
        passthrough::maybe_normalize_passthrough(
            span,
            source,
            response_value,
            payload,
            redact_messages,
        );
    })
}

#[test]
fn passthrough_input_uses_the_last_message_text() {
    let attributes = passthrough_attributes(
        r#"{"additional_args":{"complete_input_dict":{"messages":[{"role":"user","content":"first"},{"role":"assistant","content":"second"},{"role":"user","content":"third"}]}},"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        "null",
        false,
    );
    assert_eq!(attributes.get("input.value"), Some(&Value::from("third")));
    assert_eq!(
        attr(&attributes, "llm.input_messages.0.message.role"),
        Some("user")
    );
    assert_eq!(
        attr(&attributes, "llm.input_messages.1.message.content"),
        Some("second")
    );
    assert_eq!(
        attr(&attributes, "llm.input_messages.2.message.role"),
        Some("user")
    );
}

#[test]
fn passthrough_input_skips_non_object_messages() {
    let attributes = passthrough_attributes(
        r#"{"additional_args":{"complete_input_dict":{"messages":[7,{"role":"user","content":"hello"}]}},"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        "null",
        false,
    );
    assert_eq!(attributes.get("input.value"), Some(&Value::from("hello")));
    assert_eq!(attributes.get("llm.input_messages.0.message.role"), None);
    assert_eq!(
        attr(&attributes, "llm.input_messages.1.message.role"),
        Some("user")
    );
}

#[test]
fn passthrough_input_writes_empty_content_and_skips_falsy_role() {
    let attributes = passthrough_attributes(
        r#"{"additional_args":{"complete_input_dict":{"messages":[{"role":"","content":""},{"role":"user","content":null}]}},"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        "null",
        false,
    );
    assert!(!attributes.contains_key("input.value"));
    assert!(!attributes.contains_key("llm.input_messages.0.message.role"));
    assert_eq!(
        attr(&attributes, "llm.input_messages.0.message.content"),
        Some("")
    );
    assert_eq!(
        attr(&attributes, "llm.input_messages.1.message.role"),
        Some("user")
    );
    assert!(!attributes.contains_key("llm.input_messages.1.message.content"));
}

#[rstest::rstest]
#[case::empty_array(r#"{"additional_args":{"complete_input_dict":{"messages":[]}},"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#)]
#[case::object_messages(r#"{"additional_args":{"complete_input_dict":{"messages":{}}},"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#)]
#[case::missing_input(
    r#"{"additional_args":{},"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#
)]
fn passthrough_input_requires_a_non_empty_message_array(#[case] kwargs: &str) {
    assert!(passthrough_attributes(kwargs, "null", false).is_empty());
}

#[test]
fn passthrough_output_joins_content_parts_with_two_newlines() {
    let attributes = passthrough_attributes(
        r#"{"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        r#"{"role":"assistant","content":[{"type":"text","text":"one"},{"type":"text","text":"two"},{"type":"text","text":""}]}"#,
        false,
    );
    assert_eq!(attr(&attributes, "output.value"), Some("one\n\ntwo"));
    assert_eq!(
        attr(&attributes, "llm.output_messages.0.message.role"),
        Some("assistant")
    );
    assert_eq!(
        attr(&attributes, "llm.output_messages.0.message.content"),
        Some("one\n\ntwo")
    );
}

#[test]
fn passthrough_output_role_defaults_to_assistant() {
    let attributes = passthrough_attributes(
        r#"{"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        r#"{"content":[{"type":"text","text":"hi"}]}"#,
        false,
    );
    assert_eq!(
        attr(&attributes, "llm.output_messages.0.message.role"),
        Some("assistant")
    );
}

#[test]
fn passthrough_output_choices_message_is_projected() {
    let attributes = passthrough_attributes(
        r#"{"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        r#"{"choices":[{"message":{"role":"assistant","content":"choice text"}}]}"#,
        false,
    );
    assert_eq!(attr(&attributes, "output.value"), Some("choice text"));
    assert_eq!(
        attr(&attributes, "llm.output_messages.0.message.content"),
        Some("choice text")
    );
}

#[test]
fn passthrough_choices_override_the_content_output() {
    let attributes = passthrough_attributes(
        r#"{"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        r#"{"content":[{"type":"text","text":"anthropic text"}],"choices":[{"message":{"role":"assistant","content":"openai text"}}]}"#,
        false,
    );
    assert_eq!(attr(&attributes, "output.value"), Some("openai text"));
    assert_eq!(
        attr(&attributes, "llm.output_messages.0.message.content"),
        Some("openai text")
    );
}

#[rstest::rstest]
#[case::parts_without_text(
    r#"{"content":[{"type":"image","data":"abc"}],"choices":[{"message":{"content":null}}]}"#
)]
#[case::choices_with_empty_content(r#"{"choices":[{"message":{"content":""}}]}"#)]
fn passthrough_output_without_text_writes_nothing(#[case] response: &str) {
    let attributes = passthrough_attributes(
        r#"{"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        response,
        false,
    );
    assert!(attributes.is_empty());
}

#[test]
fn passthrough_response_with_a_choices_key_is_the_answer() {
    let attributes = passthrough_attributes(
        r#"{"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        r#"{"response":"ignored","choices":[{"message":{"role":"assistant","content":"direct choice"}}]}"#,
        false,
    );
    assert_eq!(attr(&attributes, "output.value"), Some("direct choice"));
}

#[rstest::rstest]
#[case::string_wrapper(
    r#"{"response":"{\"content\":[{\"type\":\"text\",\"text\":\"wrapped\"}]}"}"#,
    "wrapped"
)]
#[case::object_wrapper(
    r#"{"response":{"choices":[{"message":{"content":"wrapped"}}]}}"#,
    "wrapped"
)]
fn passthrough_wrapper_response_is_unwrapped(#[case] response: &str, #[case] text: &str) {
    let attributes = passthrough_attributes(
        r#"{"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        response,
        false,
    );
    assert_eq!(attr(&attributes, "output.value"), Some(text));
}

#[test]
fn passthrough_unparsable_wrapper_falls_back_to_original_response() {
    let attributes = passthrough_attributes(
        r#"{"original_response":{"choices":[{"message":{"content":"fallback"}}]},"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        r#"{"response":"not json"}"#,
        false,
    );
    assert_eq!(attr(&attributes, "output.value"), Some("fallback"));
}

#[test]
fn passthrough_original_response_string_is_parsed() {
    let attributes = passthrough_attributes(
        r#"{"original_response":"{\"choices\":[{\"message\":{\"content\":\"parsed fallback\"}}]}","standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        "null",
        false,
    );
    assert_eq!(attr(&attributes, "output.value"), Some("parsed fallback"));
}

#[test]
fn passthrough_response_with_content_key_is_the_answer() {
    let attributes = passthrough_attributes(
        r#"{"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        r#"{"response":{"content":"ignored"},"content":[{"type":"text","text":"direct"}]}"#,
        false,
    );
    assert_eq!(attributes.get("output.value"), Some(&Value::from("direct")));
}

#[test]
fn passthrough_redaction_skips_input_and_output() {
    let attributes = passthrough_attributes(
        r#"{"additional_args":{"complete_input_dict":{"messages":[{"role":"user","content":"secret"}]}},"original_response":{"choices":[{"message":{"content":"secret answer"}}]},"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        "null",
        true,
    );
    assert!(attributes.is_empty());
}

#[rstest::rstest]
#[case::ordinary_call_type(r#"{"call_type":"acompletion"}"#)]
#[case::empty_call_type(r#"{"call_type":""}"#)]
#[case::missing_call_type("{}")]
fn passthrough_non_passthrough_call_types_are_skipped(#[case] payload: &str) {
    let kwargs = format!(
        r#"{{"additional_args":{{"complete_input_dict":{{"messages":[{{"role":"user","content":"ignored"}}]}}}},"standard_logging_object":{payload}}}"#
    );
    assert!(passthrough_attributes(&kwargs, "null", false).is_empty());
}

#[rstest::rstest]
#[case::no_underscore("llm_passthrough_route")]
#[case::underscore("allm_pass_through_route")]
#[case::mixed_case("Anthropic_Passthrough_Route")]
fn passthrough_call_type_spellings_run(#[case] call_type: &str) {
    let kwargs = format!(
        r#"{{"additional_args":{{"complete_input_dict":{{"messages":[{{"role":"user","content":"seen"}}]}}}},"standard_logging_object":{{"call_type":"{call_type}"}}}}"#
    );
    let attributes = passthrough_attributes(&kwargs, "null", false);
    assert_eq!(attributes.get("input.value"), Some(&Value::from("seen")));
}

#[test]
fn passthrough_full_flow_overrides_the_shared_message_writes() {
    let recorded = flow(
        &map(
            r#"{"messages":[{"role":"user","content":"chat text"}],"additional_args":{"complete_input_dict":{"messages":[{"role":"system","content":"raw text"}]}},"standard_logging_object":{"call_type":"llm_passthrough_route"}}"#,
        ),
        &value(r#"{"choices":[{"message":{"content":"answer"}}]}"#),
        false,
    );
    let attributes = &recorded.attributes;
    assert_eq!(attr(attributes, "input.value"), Some("raw text"));
    assert_eq!(
        attr(attributes, "llm.input_messages.0.message.role"),
        Some("system")
    );
    assert_eq!(
        attr(attributes, "llm.input_messages.0.message.content"),
        Some("raw text")
    );
    assert_eq!(
        attr(attributes, "llm.output_messages.0.message.role"),
        Some("assistant")
    );
    assert_eq!(
        attr(attributes, "llm.output_messages.0.message.content"),
        Some("answer")
    );
}

fn mcp_attributes(kwargs: &str, response: &str, redact_messages: bool) -> BTreeMap<String, Value> {
    flow_attributes(kwargs, response, |span, _, response_value, payload| {
        mcp::maybe_set_mcp_tool_attrs(span, response_value, payload, redact_messages);
    })
}

#[test]
fn mcp_tool_name_and_arguments_are_written() {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"get_weather","arguments":{"city":"Paris"}}}}}"#,
        r#"{"content":[{"type":"text","text":"sunny"}]}"#,
        false,
    );
    assert_eq!(attr(&attributes, "tool.name"), Some("get_weather"));
    assert_eq!(
        attr(&attributes, "input.value"),
        Some(r#"{"city":"Paris"}"#)
    );
    assert_eq!(
        attr(&attributes, "input.mime_type"),
        Some("application/json")
    );
    assert_eq!(attributes.get("output.value"), Some(&Value::from("sunny")));
    assert_eq!(attr(&attributes, "output.mime_type"), Some("text/plain"));
}

#[test]
fn mcp_namespaced_tool_name_is_the_fallback() {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"","namespaced_tool_name":"server/weather"}}}}"#,
        "null",
        false,
    );
    assert_eq!(attr(&attributes, "tool.name"), Some("server/weather"));
}

#[test]
fn mcp_redaction_keeps_tool_name_and_drops_input_and_output() {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"get_weather","arguments":{"city":"Paris"}}}}}"#,
        r#"{"content":[{"type":"text","text":"sunny"}]}"#,
        true,
    );
    assert_eq!(attr(&attributes, "tool.name"), Some("get_weather"));
    assert!(!attributes.contains_key("input.value"));
    assert!(!attributes.contains_key("output.value"));
}

#[test]
fn mcp_text_only_parts_are_joined_as_plain_text() {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"tool"}}}}"#,
        r#"{"content":[{"type":"text","text":"one"},{"type":"text","text":"two"}]}"#,
        false,
    );
    assert_eq!(attr(&attributes, "output.value"), Some("one\ntwo"));
    assert_eq!(attr(&attributes, "output.mime_type"), Some("text/plain"));
}

#[test]
fn mcp_mixed_parts_are_written_as_json() {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"tool"}}}}"#,
        r#"{"content":[{"type":"text","text":"one"},{"type":"image","data":"abc"}]}"#,
        false,
    );
    assert_eq!(
        attr(&attributes, "output.value"),
        Some(r#"[{"text":"one","type":"text"},{"data":"abc","type":"image"}]"#)
    );
    assert_eq!(
        attr(&attributes, "output.mime_type"),
        Some("application/json")
    );
}

#[test]
fn mcp_part_with_empty_text_makes_the_parts_json() {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"tool"}}}}"#,
        r#"{"content":[{"type":"text","text":""}]}"#,
        false,
    );
    assert_eq!(
        attr(&attributes, "output.value"),
        Some(r#"[{"text":"","type":"text"}]"#)
    );
    assert_eq!(
        attr(&attributes, "output.mime_type"),
        Some("application/json")
    );
}

#[test]
fn mcp_structured_content_is_written_as_json() {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"tool"}}}}"#,
        r#"{"structured_content":{"temperature":72}}"#,
        false,
    );
    assert_eq!(
        attr(&attributes, "output.value"),
        Some(r#"{"temperature":72}"#)
    );
    assert_eq!(
        attr(&attributes, "output.mime_type"),
        Some("application/json")
    );
}

#[test]
fn mcp_camel_case_structured_content_is_used_when_snake_is_absent() {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"tool"}}}}"#,
        r#"{"structuredContent":{"temperature":72}}"#,
        false,
    );
    assert_eq!(
        attr(&attributes, "output.value"),
        Some(r#"{"temperature":72}"#)
    );
}

#[test]
fn mcp_null_snake_structured_content_is_not_replaced_by_camel_case() {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"tool"}}}}"#,
        r#"{"structured_content":null,"structuredContent":{"temperature":72}}"#,
        false,
    );
    assert!(!attributes.contains_key("output.value"));
}

#[test]
fn mcp_empty_content_array_is_written_as_json() {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"tool"}}}}"#,
        r#"{"content":[]}"#,
        false,
    );
    assert_eq!(attributes.get("output.value"), Some(&Value::from("[]")));
    assert_eq!(
        attr(&attributes, "output.mime_type"),
        Some("application/json")
    );
}

#[rstest::rstest]
#[case::empty_object("{}")]
#[case::null_content(r#"{"content":null}"#)]
fn mcp_without_content_or_structured_payload_writes_no_output(#[case] response: &str) {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"tool"}}}}"#,
        response,
        false,
    );
    assert!(!attributes.contains_key("output.value"));
    assert!(!attributes.contains_key("output.mime_type"));
}

#[test]
fn mcp_empty_text_content_writes_the_json_string() {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"tool"}}}}"#,
        r#"{"content":""}"#,
        false,
    );
    assert_eq!(attributes.get("output.value"), Some(&Value::from(r#""""#)));
    assert_eq!(
        attr(&attributes, "output.mime_type"),
        Some("application/json")
    );
}

#[rstest::rstest]
#[case::truthy_content(
    r#"{"content":[{"type":"text","text":"one"}],"structured_content":{"temperature":72}}"#,
    "one",
    "text/plain"
)]
#[case::empty_content_array(
    r#"{"content":[],"structured_content":{"temperature":72}}"#,
    r#"{"temperature":72}"#,
    "application/json"
)]
fn mcp_payload_prefers_truthy_content_then_structured(
    #[case] response: &str,
    #[case] value: &str,
    #[case] mime_type: &str,
) {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"tool"}}}}"#,
        response,
        false,
    );
    assert_eq!(attr(&attributes, "output.value"), Some(value));
    assert_eq!(attr(&attributes, "output.mime_type"), Some(mime_type));
}

#[test]
fn mcp_null_arguments_write_no_input() {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"tool","arguments":null}}}}"#,
        "null",
        false,
    );
    assert_eq!(attributes.get("tool.name"), Some(&Value::from("tool")));
    assert!(!attributes.contains_key("input.value"));
}

#[rstest::rstest]
#[case::other_call_type(
    r#"{"call_type":"acompletion","metadata":{"mcp_tool_call_metadata":{"name":"tool"}}}"#
)]
#[case::missing_metadata(r#"{"call_type":"call_mcp_tool"}"#)]
#[case::metadata_not_an_object(r#"{"call_type":"call_mcp_tool","metadata":"plain"}"#)]
#[case::tool_metadata_not_an_object(
    r#"{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":"plain"}}"#
)]
fn mcp_skips_non_tool_calls(#[case] payload: &str) {
    let kwargs = format!(r#"{{"standard_logging_object":{payload}}}"#);
    assert!(
        mcp_attributes(
            &kwargs,
            r#"{"content":[{"type":"text","text":"x"}]}"#,
            false
        )
        .is_empty()
    );
}

#[test]
fn mcp_response_that_is_not_an_object_writes_no_output() {
    let attributes = mcp_attributes(
        r#"{"standard_logging_object":{"call_type":"call_mcp_tool","metadata":{"mcp_tool_call_metadata":{"name":"tool"}}}}"#,
        r#""plain""#,
        false,
    );
    assert_eq!(attributes.get("tool.name"), Some(&Value::from("tool")));
    assert!(!attributes.contains_key("output.value"));
}
