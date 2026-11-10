#![cfg(test)]

use std::time::SystemTime;

use opentelemetry::trace::{Event, SpanId, Status};
use opentelemetry::{Key, KeyValue, StringValue, Value};
use opentelemetry_sdk::trace::SpanData;
use rstest::rstest;

use crate::otel::config::destination::SpanScope;
use crate::otel::fan_out::attributes::{
    LANGFUSE_TRACE_NAME, in_scope, is_llm_call_span, scoped, tenant_view,
};
use crate::otel::fan_out::tests::support::{
    PARENT_ID, attributes, keys, root_span, span, string_attribute,
};

fn error_span() -> SpanData {
    let mut span = root_span(vec![
        string_attribute("error.message", "prisma: connection refused"),
        string_attribute("error", "legacy text"),
    ]);
    span.status = Status::error("the operator's datastore is down");
    span
}

fn owned(mut span: SpanData, marker: &str) -> SpanData {
    span.attributes.push(string_attribute(marker, "owned"));
    span
}

#[rstest]
#[case::captured_request_header("http.request.header.authorization", false, false)]
#[case::captured_response_header("http.response.header.set-cookie", false, false)]
#[case::stack_trace("litellm.provider.error.stack_trace", false, false)]
#[case::url_query("url.query", false, false)]
#[case::datastore_address_on_a_database_span("server.address", true, false)]
#[case::datastore_port_on_a_database_span("server.port", true, false)]
#[case::datastore_namespace_on_a_database_span("db.namespace", true, false)]
#[case::datastore_address_elsewhere("server.address", false, true)]
#[case::datastore_namespace_elsewhere("db.namespace", false, true)]
#[case::proxy_error_text("error.message", false, false)]
#[case::proxy_error_text_legacy("error", false, false)]
fn tenant_view_withholds_operator_attributes(
    #[case] attribute: &str,
    #[case] database: bool,
    #[case] visible: bool,
) {
    let mut pairs = vec![string_attribute(attribute, "secret")];
    if database {
        pairs.push(string_attribute("db.system.name", "postgresql"));
    }
    let view = tenant_view(&span(pairs, SpanId::INVALID));
    assert_eq!(keys(&view).contains(attribute), visible);
}

#[rstest]
#[case::llm_operation("gen_ai.operation.name")]
#[case::mcp_tool("mcp.method.name")]
#[case::guardrail("litellm.guardrail.name")]
fn a_tenant_owned_span_keeps_its_error_text(#[case] marker: &str) {
    let view = tenant_view(&owned(error_span(), marker));
    assert!(keys(&view).contains("error.message"));
    assert!(keys(&view).contains("error"));
    assert_eq!(
        view.status,
        Status::error("the operator's datastore is down")
    );
}

#[test]
fn a_proxy_owned_span_loses_its_error_text_and_status_description() {
    let view = tenant_view(&error_span());
    assert!(!keys(&view).contains("error.message"));
    assert!(!keys(&view).contains("error"));
    assert_eq!(view.status, Status::error(""));
}

#[rstest]
#[case::failed_to_respond("guardrail_failed_to_respond", false)]
#[case::legacy_failure("failure", false)]
#[case::success("success", true)]
#[case::other_status("blocked", true)]
fn guardrail_response_text_follows_the_status(#[case] status: &str, #[case] visible: bool) {
    let view = tenant_view(&root_span(vec![
        string_attribute("litellm.guardrail.name", "pii"),
        string_attribute("litellm.guardrail.status", status),
        string_attribute(
            "litellm.guardrail.response",
            "http://guardrail.internal:8000 failed",
        ),
        string_attribute("error.message", "guardrail rejected the call"),
    ]));
    assert_eq!(keys(&view).contains("litellm.guardrail.response"), visible);
    assert!(keys(&view).contains("error.message"));
}

#[rstest]
#[case::new_convention(
    "http.url",
    "https://proxy.example/v1/chat?key=sk-1",
    "https://proxy.example/v1/chat"
)]
#[case::path_convention("http.target", "/v1/chat?key=sk-1", "/v1/chat")]
#[case::semconv(
    "url.full",
    "https://proxy.example/v1/chat?key=sk-1",
    "https://proxy.example/v1/chat"
)]
#[case::without_a_query(
    "http.url",
    "https://proxy.example/v1/chat",
    "https://proxy.example/v1/chat"
)]
#[case::not_a_url_key("http.route", "/v1/chat?key=sk-1", "/v1/chat?key=sk-1")]
fn tenant_view_drops_the_query_string(
    #[case] key: &str,
    #[case] given: &str,
    #[case] expected: &str,
) {
    let view = tenant_view(&root_span(vec![string_attribute(key, given)]));
    let kept = view
        .attributes
        .iter()
        .find(|pair| pair.key.as_str() == key)
        .expect("the attribute stays");
    assert_eq!(
        kept.value,
        Value::String(StringValue::from(expected.to_owned()))
    );
}

#[test]
fn a_url_key_with_a_non_string_value_is_untouched() {
    let view = tenant_view(&root_span(attributes(&[("http.url", Value::I64(7))])));
    assert_eq!(view.attributes.len(), 1);
    assert_eq!(view.attributes[0].value, Value::I64(7));
}

#[test]
fn a_proxy_owned_span_loses_every_event() {
    let mut span = root_span(vec![string_attribute("http.route", "/v1/chat")]);
    span.events.events.push(Event::new(
        "exception",
        SystemTime::UNIX_EPOCH,
        vec![string_attribute("exception.message", "boom")],
        0,
    ));
    span.events.dropped_count = 4;
    let view = tenant_view(&span);
    assert!(view.events.events.is_empty());
    assert_eq!(view.events.dropped_count, 0);
}

#[test]
fn a_tenant_owned_span_loses_only_the_stack_trace_of_its_events() {
    let mut span = owned(
        root_span(vec![string_attribute("http.route", "/v1/chat")]),
        "gen_ai.operation.name",
    );
    span.events.events.push(Event::new(
        "exception",
        SystemTime::UNIX_EPOCH,
        vec![
            string_attribute("exception.message", "boom"),
            string_attribute("exception.stacktrace", "at operator.py:1"),
        ],
        3,
    ));
    let view = tenant_view(&span);
    let events = &view.events.events;
    assert_eq!(events.len(), 1);
    assert!(
        events[0]
            .attributes
            .iter()
            .all(|pair| pair.key.as_str() != "exception.stacktrace")
    );
    assert_eq!(events[0].dropped_attributes_count, 3);
}

#[test]
fn a_changed_view_reports_no_dropped_attributes_events_or_links() {
    let mut span = owned(
        root_span(vec![string_attribute("http.route", "/v1/chat")]),
        "gen_ai.operation.name",
    );
    span.events.events.push(Event::new(
        "exception",
        SystemTime::UNIX_EPOCH,
        vec![string_attribute("exception.stacktrace", "at operator.py:1")],
        2,
    ));
    span.dropped_attributes_count = 9;
    span.events.dropped_count = 2;
    span.links.dropped_count = 1;
    let view = tenant_view(&span);
    assert_eq!(view.dropped_attributes_count, 0);
    assert_eq!(view.events.dropped_count, 0);
    assert_eq!(view.links.dropped_count, 0);
}

#[test]
fn an_untouched_span_keeps_its_dropped_counts() {
    let mut span = owned(
        root_span(vec![string_attribute("http.route", "/v1/chat")]),
        "gen_ai.operation.name",
    );
    span.dropped_attributes_count = 9;
    span.events.dropped_count = 2;
    span.links.dropped_count = 1;
    assert_eq!(tenant_view(&span), span);
}

#[test]
fn an_untouched_span_with_an_ordinary_event_keeps_its_dropped_counts() {
    let mut span = owned(
        root_span(vec![string_attribute("http.route", "/v1/chat")]),
        "gen_ai.operation.name",
    );
    span.events.events.push(Event::new(
        "log",
        SystemTime::UNIX_EPOCH,
        vec![string_attribute("exception.message", "boom")],
        0,
    ));
    span.dropped_attributes_count = 9;
    assert_eq!(tenant_view(&span), span);
}

#[test]
fn an_owned_span_still_loses_the_query_string() {
    let view = tenant_view(&owned(
        root_span(vec![string_attribute(
            "http.url",
            "https://provider.example/v1/chat?api-version=2024",
        )]),
        "gen_ai.operation.name",
    ));
    let kept = view
        .attributes
        .iter()
        .find(|pair| pair.key.as_str() == "http.url")
        .expect("the attribute stays");
    assert_eq!(
        kept.value,
        Value::String(StringValue::from(
            "https://provider.example/v1/chat".to_owned()
        ))
    );
}

#[test]
fn full_scope_returns_the_span_unchanged() {
    let span = span(
        vec![string_attribute("http.route", "/v1/chat")],
        SpanId::from(PARENT_ID),
    );
    assert_eq!(scoped(span.clone(), SpanScope::Full), span);
}

#[test]
fn llm_only_names_a_root_model_call_and_keeps_it_a_root() {
    let view = scoped(
        root_span(vec![string_attribute("gen_ai.operation.name", "chat")]),
        SpanScope::LlmOnly,
    );
    assert_eq!(view.parent_span_id, SpanId::INVALID);
    let name = view
        .attributes
        .iter()
        .find(|pair| pair.key.as_str() == LANGFUSE_TRACE_NAME)
        .expect("the trace is named");
    assert_eq!(name.value, Value::from("call model".to_owned()));
}

#[test]
fn llm_only_roots_a_child_model_call_and_names_the_trace() {
    let view = scoped(
        span(
            vec![string_attribute("gen_ai.operation.name", "chat")],
            SpanId::from(PARENT_ID),
        ),
        SpanScope::LlmOnly,
    );
    assert_eq!(view.parent_span_id, SpanId::INVALID);
    assert!(!view.parent_span_is_remote);
    assert!(
        view.attributes
            .iter()
            .any(|pair| pair.key.as_str() == LANGFUSE_TRACE_NAME)
    );
}

#[test]
fn llm_only_leaves_an_already_named_root_alone() {
    let mut span = root_span(vec![string_attribute("gen_ai.operation.name", "chat")]);
    span.attributes
        .push(string_attribute(LANGFUSE_TRACE_NAME, "checkout"));
    span.dropped_attributes_count = 5;
    let view = scoped(span.clone(), SpanScope::LlmOnly);
    assert_eq!(view, span);
    assert_eq!(
        view.attributes
            .iter()
            .filter(|pair| pair.key.as_str() == LANGFUSE_TRACE_NAME)
            .count(),
        1
    );
}

#[test]
fn llm_only_keeps_a_named_trace_name_on_a_child() {
    let mut child = span(vec![], SpanId::from(PARENT_ID));
    child
        .attributes
        .push(string_attribute(LANGFUSE_TRACE_NAME, "checkout"));
    let view = scoped(child, SpanScope::LlmOnly);
    assert_eq!(view.parent_span_id, SpanId::INVALID);
    let name = view
        .attributes
        .iter()
        .find(|pair| pair.key.as_str() == LANGFUSE_TRACE_NAME)
        .expect("the name stays");
    assert_eq!(name.value, Value::from("checkout".to_owned()));
}

#[rstest]
#[case::model_call(vec![("gen_ai.operation.name", "chat")], true, true)]
#[case::mcp_tool(vec![("gen_ai.operation.name", "execute_tool"), ("mcp.method.name", "tools/call")], false, false)]
#[case::request_root(vec![("http.route", "/v1/chat")], false, false)]
fn is_llm_call_span_tells_model_calls_from_the_rest(
    #[case] pairs: Vec<(&str, &str)>,
    #[case] llm_call: bool,
    #[case] in_llm_only: bool,
) {
    let materialized: Vec<KeyValue> = pairs
        .into_iter()
        .map(|(key, value)| KeyValue::new(Key::new(key.to_owned()), Value::from(value.to_owned())))
        .collect();
    let span = root_span(materialized);
    assert_eq!(is_llm_call_span(&span), llm_call);
    assert_eq!(in_scope(&span, SpanScope::LlmOnly), in_llm_only);
    assert!(in_scope(&span, SpanScope::Full));
}
