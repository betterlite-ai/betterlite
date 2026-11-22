use betterlite_core::api::request_principal_metadata::Metadata;
use betterlite_logging::otel::plumbing::context::context_slots::RequestTraceContext;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use http::header::HeaderValue;
use indexmap::IndexMap;
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};

use crate::passthrough::metadata_tags::{update_metadata_tags, with_trace_context};

fn data(text: &str) -> Metadata {
    serde_json::from_str(text).unwrap()
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn headers(name: &'static str, value: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(name, HeaderValue::from_static(value));
    headers
}

#[test]
fn passthrough_update_metadata_tags() {
    let mut metadata = data(r#"{"team_tag": "t1"}"#);
    update_metadata_tags(&HeaderMap::new(), &mut metadata);
    assert!(metadata.get("tags").is_none());
    assert_eq!(metadata["team_tag"], Json::Text("t1".to_owned()));

    let mut both = HeaderMap::new();
    both.insert("tags", HeaderValue::from_static("one, two"));
    both.insert("x-litellm-tags", HeaderValue::from_static(" three ,four"));
    update_metadata_tags(&both, &mut metadata);
    assert_eq!(metadata["tags"], json(r#"["one", "two", "three", "four"]"#));

    // an existing list is extended in place
    let mut existing = data(r#"{"tags": ["old"]}"#);
    update_metadata_tags(&headers("x-litellm-tags", "new"), &mut existing);
    assert_eq!(existing["tags"], json(r#"["old", "new"]"#));

    // a tags value that is not a list becomes one, rather than failing the request
    let mut replaced = data(r#"{"tags": "scalar"}"#);
    update_metadata_tags(&headers("tags", "one"), &mut replaced);
    assert_eq!(replaced["tags"], json(r#"["one"]"#));

    // an empty header value is no tags at all, the way an empty Python string is falsy
    let mut empty = data("{}");
    update_metadata_tags(&headers("tags", ""), &mut empty);
    assert!(empty.get("tags").is_none());
}

#[test]
fn passthrough_with_trace_context() {
    let trace = RequestTraceContext {
        root: None,
        mcp_carrier: None,
        mcp_transport: None,
        destinations: Vec::new(),
    };
    let headers: IndexMap<String, String> = [
        ("x-a".to_owned(), "1".to_owned()),
        ("traceparent".to_owned(), "stale".to_owned()),
    ]
    .into_iter()
    .collect();

    let span = SpanContext::new(
        TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").unwrap(),
        SpanId::from_hex("b7ad6b7169203331").unwrap(),
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    );
    let injected = with_trace_context(&headers, Some(&span), &trace);
    assert_eq!(injected["x-a"], "1");
    assert_eq!(
        injected["traceparent"],
        "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
    );

    // with no span to propagate the headers come back as a copy, untouched
    let copied = with_trace_context(&headers, None, &trace);
    assert_eq!(copied, headers);
}
