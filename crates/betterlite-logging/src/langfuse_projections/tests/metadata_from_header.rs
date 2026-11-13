#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::metadata_header::add_metadata_from_header;
use super::support::{capture_logs, value};

fn litellm_params(headers: &str) -> Json {
    value(&format!(
        r#"{{"proxy_server_request": {{"headers": {headers}}}}}"#
    ))
}

fn metadata(pairs: &[(&str, &str)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), Json::Text((*value).to_owned())))
        .collect()
}

#[test]
fn metadata_from_header_injects_langfuse_headers_without_the_prefix() {
    let params = litellm_params(r#"{"langfuse_trace_id": "trace-1", "x-other": "ignored"}"#);
    let merged = add_metadata_from_header(Some(&params), metadata(&[("user", "max")]));
    assert_eq!(
        merged.get("trace_id"),
        Some(&Json::Text("trace-1".to_owned()))
    );
    assert_eq!(merged.get("user"), Some(&Json::Text("max".to_owned())));
    assert_eq!(merged.get("x-other"), None);
    assert_eq!(merged.get("langfuse_trace_id"), None);
}

#[test]
fn header_overwrite_replaces_the_existing_metadata_value() {
    let params = litellm_params(r#"{"langfuse_trace_id": "from-header"}"#);
    let merged =
        add_metadata_from_header(Some(&params), metadata(&[("trace_id", "from-metadata")]));
    assert_eq!(
        merged.get("trace_id"),
        Some(&Json::Text("from-header".to_owned()))
    );
}

#[test]
fn header_warn_reports_an_overwrite_and_stays_quiet_for_a_new_key() {
    let (logs, _guard) = capture_logs();
    let overwrite = add_metadata_from_header(
        Some(&litellm_params(r#"{"langfuse_trace_id": "from-header"}"#)),
        metadata(&[("trace_id", "from-metadata")]),
    );
    assert_eq!(
        overwrite.get("trace_id"),
        Some(&Json::Text("from-header".to_owned()))
    );
    let text = logs.text();
    assert!(
        text.contains("Overwriting Langfuse `trace_id` from request header"),
        "{text}"
    );

    let (fresh_logs, _fresh_guard) = capture_logs();
    let fresh = add_metadata_from_header(
        Some(&litellm_params(r#"{"langfuse_session_id": "session-1"}"#)),
        metadata(&[]),
    );
    assert_eq!(
        fresh.get("session_id"),
        Some(&Json::Text("session-1".to_owned()))
    );
    assert!(!fresh_logs.text().contains("Overwriting Langfuse"));
}

#[test]
fn metadata_from_header_without_a_proxy_request_returns_the_metadata() {
    let params = value(r#"{"metadata": {}}"#);
    let original = metadata(&[("user", "max")]);
    assert_eq!(
        add_metadata_from_header(Some(&params), original.clone()),
        original
    );
    assert_eq!(add_metadata_from_header(None, original.clone()), original);
}
