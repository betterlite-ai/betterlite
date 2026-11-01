use betterlite_logging::rubrik::request_data::{
    RequestContextSource, ResponseSource, TrackedField, apply_correlation_id, correlation_id,
    extract_request_data, resolve_model,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

#[test]
fn extract_request_data_returns_none_when_both_sources_are_absent() {
    assert_eq!(extract_request_data(None, None), None);
}

#[test]
fn extract_request_data_falls_back_to_optional_params_tools_when_request_data_lacks_the_key() {
    let call_details = RequestContextSource {
        messages: Some(Json::Text("hi".to_owned())),
        model: Some("gpt-4".to_owned()),
        tools: TrackedField::Missing,
        optional_params_tools: Some(Json::List(vec![Json::Text("search".to_owned())])),
    };
    let payload = extract_request_data(Some(&call_details), None).expect("data present");
    assert_eq!(
        payload.tools,
        Some(Json::List(vec![Json::Text("search".to_owned())]))
    );
    assert_eq!(payload.model, Some("gpt-4".to_owned()));
}

#[test]
fn extract_request_data_forwards_an_explicit_empty_tools_list_from_request_data() {
    let call_details = RequestContextSource {
        messages: None,
        model: None,
        tools: TrackedField::Missing,
        optional_params_tools: Some(Json::List(vec![Json::Text("search".to_owned())])),
    };
    let request_data = RequestContextSource {
        messages: None,
        model: None,
        tools: TrackedField::Present(Json::List(vec![])),
        optional_params_tools: None,
    };
    let payload =
        extract_request_data(Some(&call_details), Some(&request_data)).expect("data present");
    assert_eq!(payload.tools, Some(Json::List(vec![])));
}

#[test]
fn resolve_model_prefers_the_response_model_when_the_response_has_one() {
    let response = ResponseSource::WithModel(Some("claude-3".to_owned()));
    assert_eq!(resolve_model(&response, Some("fallback")), "claude-3");
}

#[test]
fn resolve_model_falls_back_to_unknown_when_the_response_model_is_empty() {
    let response = ResponseSource::WithModel(Some(String::new()));
    assert_eq!(resolve_model(&response, Some("fallback")), "unknown");
}

#[test]
fn resolve_model_uses_call_details_model_as_given_even_when_it_is_empty() {
    let response = ResponseSource::Absent;
    assert_eq!(resolve_model(&response, Some("")), "");
}

#[test]
fn resolve_model_defaults_to_unknown_when_neither_source_has_a_model() {
    assert_eq!(resolve_model(&ResponseSource::Absent, None), "unknown");
    assert_eq!(
        resolve_model(&ResponseSource::WithoutModel, None),
        "unknown"
    );
}

#[test]
fn correlation_id_prefers_call_details_then_falls_back_to_request_data() {
    assert_eq!(
        correlation_id(Some("call-1"), Some("call-2")),
        Some("call-1".to_owned())
    );
    assert_eq!(
        correlation_id(None, Some("call-2")),
        Some("call-2".to_owned())
    );
    assert_eq!(
        correlation_id(Some(""), Some("call-2")),
        Some("call-2".to_owned())
    );
}

#[test]
fn correlation_id_is_none_when_both_sources_are_missing() {
    assert_eq!(correlation_id(None, None), None);
}

#[test]
fn apply_correlation_id_overwrites_the_id_field_in_place_when_present() {
    let mut payload = IndexMap::from([("id".to_owned(), Json::Text("old-id".to_owned()))]);
    apply_correlation_id(&mut payload, Some("call-1"));
    assert_eq!(payload.get("id"), Some(&Json::Text("call-1".to_owned())));
}

#[test]
fn apply_correlation_id_leaves_the_id_field_unchanged_when_correlation_is_absent() {
    let mut payload = IndexMap::from([("id".to_owned(), Json::Text("old-id".to_owned()))]);
    apply_correlation_id(&mut payload, None);
    assert_eq!(payload.get("id"), Some(&Json::Text("old-id".to_owned())));
}
