use betterlite_logging::spend_bodies::session::SpendTimestamp;
use betterlite_models::decisions::Json;
use chrono::DateTime;
use serde_json::Number;

use super::super::fields::{
    CallLog, completion_start, eligible_route, failure_start_time, lift_call_log,
    request_tags_for_cost_tracking,
};
use super::super::metadata::{
    as_object, is_truthy, merge_failure_metadata, used_client_oauth_token,
};
use super::support::object;
use crate::spend::payload::CompletionStart;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn aware(value: &str) -> SpendTimestamp {
    SpendTimestamp::Aware(DateTime::parse_from_rfc3339(value).expect("the fixture is rfc3339"))
}

#[test]
fn falsey_values_are_not_present() {
    assert!(!is_truthy(&Json::Null));
    assert!(!is_truthy(&Json::Bool(false)));
    assert!(!is_truthy(&Json::Number(Number::from(0_u8))));
    assert!(!is_truthy(&Json::Number(
        Number::from_f64(0.0).expect("zero is finite")
    )));
    assert!(!is_truthy(&text("")));
    assert!(!is_truthy(&Json::List(Vec::new())));
    assert!(!is_truthy(&Json::Object(object("{}"))));
}

#[test]
fn truthy_values_are_present() {
    assert!(is_truthy(&Json::Bool(true)));
    assert!(is_truthy(&Json::Number(Number::from(1_u8))));
    assert!(is_truthy(&text("x")));
    assert!(is_truthy(&Json::List(vec![Json::Bool(true)])));
    assert!(is_truthy(&Json::Object(object(r#"{"a": 1}"#))));
}

#[test]
fn oauth_provenance_reads_only_a_bool_from_its_bucket() {
    assert_eq!(
        used_client_oauth_token(
            &object(r#"{"metadata": {"used_client_oauth_token": true}}"#),
            None
        ),
        Some(true)
    );
    assert_eq!(
        used_client_oauth_token(
            &object(r#"{"metadata": {"used_client_oauth_token": false}}"#),
            None
        ),
        Some(false)
    );
    assert_eq!(
        used_client_oauth_token(
            &object(r#"{"metadata": {"used_client_oauth_token": "true"}}"#),
            None
        ),
        None
    );
    assert_eq!(
        used_client_oauth_token(
            &object(r#"{"metadata": {"used_client_oauth_token": true}}"#),
            Some("/v1/messages")
        ),
        None
    );
}

#[test]
fn a_null_guardrail_source_is_not_copied() {
    let mut request_data =
        object(r#"{"litellm_metadata": {"standard_logging_guardrail_information": null}}"#);
    merge_failure_metadata(&mut request_data, object("{}"));
    let metadata = as_object(&request_data["litellm_params"]["metadata"]);
    assert!(
        !metadata
            .is_some_and(|fields| fields.contains_key("standard_logging_guardrail_information"))
    );
}

#[test]
fn an_absent_proxy_request_becomes_an_empty_object() {
    let mut request_data = object("{}");
    merge_failure_metadata(&mut request_data, object("{}"));
    assert_eq!(
        request_data["litellm_params"]["proxy_server_request"],
        Json::Object(object("{}"))
    );
}

#[test]
fn merge_fills_an_absent_model_and_provider() {
    let mut request_data =
        object(r#"{"litellm_params": {"model": "gpt-4o", "custom_llm_provider": "openai"}}"#);
    merge_failure_metadata(&mut request_data, object("{}"));
    assert_eq!(request_data["model"], text("gpt-4o"));
    assert_eq!(request_data["custom_llm_provider"], text("openai"));
}

#[test]
fn request_tags_prefer_a_list_valued_payload() {
    let empty = Json::Object(object(r#"{"request_tags": []}"#));
    let metadata = Json::Object(object(r#"{"tags": ["metadata"]}"#));
    assert_eq!(
        request_tags_for_cost_tracking(Some(&empty), Some(&metadata)),
        Some(&Json::List(Vec::new()))
    );
    let text_tags = Json::Object(object(r#"{"request_tags": "payload"}"#));
    assert_eq!(
        request_tags_for_cost_tracking(Some(&text_tags), Some(&metadata)),
        Some(&Json::List(vec![text("metadata")]))
    );
    assert_eq!(
        request_tags_for_cost_tracking(None, Some(&Json::Object(object("{}")))),
        None
    );
}

#[test]
fn completion_start_reads_only_a_datetime_value() {
    let request = object(r#"{"completion_start_time": "2024-01-01T00:00:00Z"}"#);
    assert!(matches!(completion_start(&request), CompletionStart::At(_)));
    let absent = object(r#"{"completion_start_time": "soon"}"#);
    assert!(matches!(completion_start(&absent), CompletionStart::Absent));
}

#[test]
fn lift_replaces_a_falsey_object_and_fills_a_null_trace() {
    let mut request_data = object(r#"{"standard_logging_object": {}, "litellm_trace_id": null}"#);
    let call_log = CallLog {
        standard_logging_object: Some(Json::Object(object(r#"{"id": "call"}"#))),
        litellm_trace_id: Some("trace".to_owned()),
        start_time: None,
    };
    lift_call_log(&mut request_data, &call_log);
    assert_eq!(
        request_data["standard_logging_object"],
        Json::Object(object(r#"{"id": "call"}"#))
    );
    assert_eq!(request_data["litellm_trace_id"], text("trace"));
}

#[test]
fn llm_and_info_routes_are_eligible_and_others_are_not() {
    assert!(eligible_route(None));
    assert!(eligible_route(Some("/v1/chat/completions")));
    assert!(eligible_route(Some("/health")));
    assert!(!eligible_route(Some("/v1/unknown")));
}

#[test]
fn a_datetime_request_start_beats_the_clock() {
    let request = object(r#"{"start_time": "2024-01-01T00:00:00Z"}"#);
    let start = failure_start_time(&request, None, aware("2024-09-09T09:09:09Z"));
    assert_eq!(start, aware("2024-01-01T00:00:00Z"));
}
