#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_logging::opik::metadata::{
    apply_proxy_header_overrides, extract_opik_metadata, extract_span_identifiers, extract_tags,
    normalize_provider_name,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

fn obj(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("valid fixture JSON")
}

fn val(text: &str) -> Json {
    serde_json::from_str(text).expect("valid fixture JSON")
}

#[test]
fn metadata_priority_provider_and_proxy_tags() {
    let auth = obj(
        r#"{"user_api_key_auth_metadata":{"opik":{"project_name":"auth","tags":["a"]}},"requester_metadata":{"opik":{"project_name":"requester"}}}"#,
    );
    let request = obj(r#"{"opik":{"project_name":"request"}}"#);
    let merged = extract_opik_metadata(&request, &auth);
    assert_eq!(merged["project_name"], val(r#""requester""#));
    assert_eq!(
        normalize_provider_name(Some("gemini")),
        Some(String::from("google_ai"))
    );
    let (project, tags, thread) = apply_proxy_header_overrides(
        String::from("p"),
        extract_tags(&merged, Some("gemini")),
        None,
        &BTreeMap::from([(String::from("opik_tags"), String::from("[\"b\"]"))]),
    );
    assert_eq!(project, "p");
    assert_eq!(tags, vec![val(r#""a""#), val(r#""gemini""#), val(r#""b""#)]);
    assert_eq!(thread, None);
}

#[test]
fn span_identifiers_accept_dict_and_invalid_values() {
    assert_eq!(
        extract_span_identifiers(Some(&val(r#"{"trace_id": "trace", "id": "span"}"#))),
        (Some(String::from("trace")), Some(String::from("span")))
    );
    assert_eq!(
        extract_span_identifiers(Some(&val(r#""invalid""#))),
        (None, None)
    );
    assert_eq!(extract_span_identifiers(None), (None, None));
}

#[test]
fn malformed_proxy_tags_are_ignored() {
    let (project, tags, thread) = apply_proxy_header_overrides(
        String::from("p"),
        vec![val(r#""existing""#)],
        Some(String::from("old-thread")),
        &BTreeMap::from([
            (String::from("opik_tags"), String::from("not-json")),
            (String::from("opik_project_name"), String::from("new-p")),
            (String::from("opik_thread_id"), String::from("new-thread")),
        ]),
    );
    assert_eq!(project, "new-p");
    assert_eq!(tags, vec![val(r#""existing""#)]);
    assert_eq!(thread, Some(String::from("new-thread")));
}
