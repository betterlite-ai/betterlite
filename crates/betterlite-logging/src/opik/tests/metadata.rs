#![cfg(test)]

use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn metadata_priority_provider_and_proxy_tags() {
    let auth = json!({"user_api_key_auth_metadata":{"opik":{"project_name":"auth","tags":["a"]}},"requester_metadata":{"opik":{"project_name":"requester"}}});
    let request = json!({"opik":{"project_name":"request"}});
    let merged = betterlite_logging::opik::extract_opik_metadata(
        request.as_object().unwrap(),
        auth.as_object().unwrap(),
    );
    assert_eq!(merged["project_name"], "requester");
    assert_eq!(
        betterlite_logging::opik::normalize_provider_name(Some("gemini")),
        Some(String::from("google_ai"))
    );
    let (project, tags, thread) = betterlite_logging::opik::apply_proxy_header_overrides(
        String::from("p"),
        betterlite_logging::opik::extract_tags(&merged, Some("gemini")),
        None,
        &BTreeMap::from([(String::from("opik_tags"), String::from("[\"b\"]"))]),
    );
    assert_eq!(project, "p");
    assert_eq!(tags, vec![json!("a"), json!("gemini"), json!("b")]);
    assert_eq!(thread, None);
}

#[test]
fn span_identifiers_accept_dict_and_invalid_values() {
    assert_eq!(
        betterlite_logging::opik::extract_span_identifiers(Some(
            &json!({"trace_id": "trace", "id": "span"})
        )),
        (Some(String::from("trace")), Some(String::from("span")))
    );
    assert_eq!(
        betterlite_logging::opik::extract_span_identifiers(Some(&json!("invalid"))),
        (None, None)
    );
    assert_eq!(
        betterlite_logging::opik::extract_span_identifiers(None),
        (None, None)
    );
}

#[test]
fn malformed_proxy_tags_are_ignored() {
    let (project, tags, thread) = betterlite_logging::opik::apply_proxy_header_overrides(
        String::from("p"),
        vec![json!("existing")],
        Some(String::from("old-thread")),
        &BTreeMap::from([
            (String::from("opik_tags"), String::from("not-json")),
            (String::from("opik_project_name"), String::from("new-p")),
            (String::from("opik_thread_id"), String::from("new-thread")),
        ]),
    );
    assert_eq!(project, "new-p");
    assert_eq!(tags, vec![json!("existing")]);
    assert_eq!(thread, Some(String::from("new-thread")));
}
