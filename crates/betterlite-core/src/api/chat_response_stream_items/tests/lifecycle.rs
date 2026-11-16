use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::chat_response_stream_items::error::StreamItemsError;
use crate::api::chat_response_stream_items::lifecycle::{
    create_response_created_event, create_response_in_progress_event,
};
use crate::api::chat_response_stream_items::progression::return_default_initial_events;
use crate::api::chat_response_stream_tools::error::StreamToolsError;
use crate::api::response_id_codecs::response::build_response_id;

use super::fixtures::{event_json, field, json, sources, state_with, state_with_details};

const RAW_ID: &str = "resp_u";
const DEFAULTS_ID: &str =
    "resp_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOk5vbmU7bW9kZWxfaWQ6Tm9uZTtyZXNwb25zZV9pZDpyZXNwX3U=";
const AZURE_ID: &str = "resp_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOmF6dXJlO21vZGVsX2lkOmRlcGxveW1lbnQtMTtyZXNwb25zZV9pZDpyZXNwX3U=";

fn expected_response(id: &str, created_at: i64) -> Json {
    let text = format!(
        r#"{{"id":"{id}","object":"response","created_at":{created_at},"status":"in_progress","error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"model":"gpt-4o-mini","output":[],"parallel_tool_calls":true,"previous_response_id":null,"reasoning":{{"effort":null,"summary":null}},"store":true,"temperature":null,"text":null,"tool_choice":"auto","tools":[],"top_p":1.0,"truncation":null,"usage":null,"user":null,"metadata":null}}"#
    );
    json(&text)
}

fn expected_event(kind: &str, sequence: i64, response: Json) -> Json {
    let mut event = IndexMap::new();
    event.insert("type".to_owned(), Json::Text(kind.to_owned()));
    event.insert(
        "sequence_number".to_owned(),
        Json::Number(serde_json::Number::from(sequence)),
    );
    event.insert("response".to_owned(), response);
    Json::Object(event)
}

fn with_fields(response: Json, overrides: &[(&str, &str)]) -> Json {
    let Json::Object(mut fields) = response else {
        panic!("the expected response is an object");
    };
    for (name, text) in overrides {
        fields.insert((*name).to_owned(), json(text));
    }
    Json::Object(fields)
}

fn metadata_with(model_info: Json) -> IndexMap<String, Json> {
    let mut metadata = IndexMap::new();
    metadata.insert("model_info".to_owned(), model_info);
    metadata
}

#[test]
fn lifecycle_defaults() {
    let mut state = state_with("{}");
    let sources = sources(10, &["u"]);
    let created = return_default_initial_events(&mut state, &sources)
        .expect("the created event builds")
        .expect("the first call creates the response");
    assert_eq!(
        event_json(&created),
        expected_event("response.created", 1, expected_response(DEFAULTS_ID, 10))
    );
    let in_progress = return_default_initial_events(&mut state, &sources)
        .expect("the in-progress event builds")
        .expect("the second call starts the response");
    assert_eq!(
        event_json(&in_progress),
        expected_event(
            "response.in_progress",
            2,
            expected_response(DEFAULTS_ID, 10)
        )
    );
    assert!(
        return_default_initial_events(&mut state, &sources)
            .expect("the third call succeeds")
            .is_none()
    );
    assert_eq!(state.progress.cached_response_id.as_deref(), Some(RAW_ID));
    assert_eq!(state.progress.sequence_number, 2);
    assert!(state.progress.sent_response_created);
    assert!(state.progress.sent_in_progress);
}

#[test]
fn lifecycle_copies_request_fields_even_when_null() {
    let request = r#"{"instructions":null,"temperature":null,"text":null,"tools":null,"top_p":null,"truncation":null,"user":null,"metadata":null}"#;
    let mut state = state_with(request);
    let created = create_response_created_event(&mut state, &sources(10, &["u"]))
        .expect("the created event builds");
    let response = field(&event_json(&created), "response");
    let expected = with_fields(
        expected_response(DEFAULTS_ID, 10),
        &[("tools", "null"), ("top_p", "null")],
    );
    assert_eq!(response, expected);
}

#[test]
fn lifecycle_copies_supplied_request_fields() {
    let request = r#"{"instructions":"be brief","temperature":0.2,"text":{"format":{"type":"text"}},"tools":[],"top_p":0.9,"truncation":"disabled","user":"grace","metadata":{"trace":"t1"},"tool_choice":"required"}"#;
    let mut state = state_with(request);
    let created = create_response_created_event(&mut state, &sources(10, &["u"]))
        .expect("the created event builds");
    let response = field(&event_json(&created), "response");
    let expected = with_fields(
        expected_response(DEFAULTS_ID, 10),
        &[
            ("instructions", "\"be brief\""),
            ("temperature", "0.2"),
            ("text", r#"{"format":{"type":"text"}}"#),
            ("tool_choice", "\"required\""),
            ("top_p", "0.9"),
            ("truncation", "\"disabled\""),
            ("user", "\"grace\""),
            ("metadata", r#"{"trace":"t1"}"#),
        ],
    );
    assert_eq!(response, expected);
}

#[test]
fn lifecycle_keeps_whole_numbers_written_whole() {
    let mut state = state_with(r#"{"top_p":1,"temperature":0}"#);
    let created = create_response_created_event(&mut state, &sources(10, &["u"]))
        .expect("the created event builds");
    let response = field(&event_json(&created), "response");
    let expected = with_fields(
        expected_response(DEFAULTS_ID, 10),
        &[("top_p", "1"), ("temperature", "0")],
    );
    assert_eq!(response, expected);
}

#[test]
fn lifecycle_ignores_request_values_with_fixed_defaults() {
    let request = r#"{"max_output_tokens":99,"parallel_tool_calls":false,"previous_response_id":"prev","reasoning":{"effort":"high"},"store":false}"#;
    let mut state = state_with(request);
    let created = create_response_created_event(&mut state, &sources(10, &["u"]))
        .expect("the created event builds");
    let response = field(&event_json(&created), "response");
    assert_eq!(response, expected_response(DEFAULTS_ID, 10));
}

#[test]
fn lifecycle_encodes_ids_with_provider_and_model_metadata() {
    let mut model_info = IndexMap::new();
    model_info.insert("id".to_owned(), Json::Text("deployment-1".to_owned()));
    let metadata = metadata_with(Json::Object(model_info));
    let mut state = state_with_details("{}", Some("azure"), Some(metadata));
    let created = create_response_created_event(&mut state, &sources(11, &["u"]))
        .expect("the created event builds");
    let response = field(&event_json(&created), "response");
    assert_eq!(
        response,
        expected_response(AZURE_ID, 11),
        "the encoded id carries the provider and the model id"
    );
    assert_eq!(state.progress.cached_response_id.as_deref(), Some(RAW_ID));
}

#[test]
fn lifecycle_encodes_ids_when_the_metadata_has_no_model_id() {
    let mut state = state_with_details("{}", None, Some(metadata_with(json(r#"{"id":null}"#))));
    let created = create_response_in_progress_event(&mut state, &sources(10, &["u"]))
        .expect("the in-progress event builds");
    assert_eq!(
        field(&field(&event_json(&created), "response"), "id"),
        Json::Text(DEFAULTS_ID.to_owned())
    );
}

#[test]
fn lifecycle_keeps_an_encoded_cached_id() {
    let encoded = build_response_id(Some("azure"), Some("deployment-1"), RAW_ID);
    let mut state = state_with("{}");
    state.progress.cached_response_id = Some(encoded.clone());
    let created = create_response_created_event(&mut state, &sources(10, &["unused"]))
        .expect("the created event builds");
    assert_eq!(
        field(&field(&event_json(&created), "response"), "id"),
        Json::Text(encoded)
    );
}

#[test]
fn lifecycle_rejects_metadata_that_is_not_a_model_id() {
    let mut wrong_info = state_with_details("{}", None, Some(metadata_with(json("\"m\""))));
    let info_error = create_response_created_event(&mut wrong_info, &sources(10, &["u"]));
    assert!(matches!(
        info_error,
        Err(StreamItemsError::Metadata {
            field: "litellm_metadata.model_info"
        })
    ));
    let mut wrong_id = state_with_details("{}", None, Some(metadata_with(json(r#"{"id":7}"#))));
    let id_error = create_response_created_event(&mut wrong_id, &sources(10, &["u"]));
    assert!(matches!(
        id_error,
        Err(StreamItemsError::Metadata {
            field: "litellm_metadata.model_info.id"
        })
    ));
}

#[test]
fn lifecycle_rejects_a_request_that_is_not_an_object() {
    let mut state = state_with("[]");
    let result = create_response_created_event(&mut state, &sources(10, &["u"]));
    assert!(matches!(result, Err(StreamItemsError::ResponseNotObject)));
}

#[test]
fn lifecycle_reports_request_fields_of_the_wrong_type() {
    let mut state = state_with(r#"{"instructions":5}"#);
    let result = create_response_created_event(&mut state, &sources(10, &["u"]));
    assert!(matches!(result, Err(StreamItemsError::Response { .. })));
}

#[test]
fn lifecycle_reports_sequence_overflow() {
    let mut state = state_with("{}");
    state.progress.sequence_number = i64::MAX;
    let result = create_response_created_event(&mut state, &sources(10, &["u"]));
    assert!(matches!(
        result,
        Err(StreamItemsError::Stream(StreamToolsError::SequenceOverflow))
    ));
}
