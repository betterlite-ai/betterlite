use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use serde_json::json;

use super::{
    MODEL, OPERATION, PROVIDER, encoded, number, object, request_data, settings, text, utc,
};
use crate::vertex_veo_operation::error::VeoOperationError;
use crate::vertex_veo_operation::response::{create_response, edit_response, status_response};

#[test]
fn a_failed_operation_beats_done() {
    let body = encoded(&json!({
        "name": OPERATION,
        "done": true,
        "error": {"code": 13, "message": "boom"},
    }));
    let video = status_response(&body, Some(PROVIDER), 7, &utc()).expect("the status converts");
    assert_eq!(video.status, "failed");
    assert_eq!(
        video.id,
        encode_video_id_with_provider(OPERATION, PROVIDER, Some(MODEL))
    );
    assert_eq!(video.model.as_deref(), Some(MODEL));
    assert_eq!(
        serde_json::to_value(&video.error).expect("the error converts"),
        json!({"code": 13, "message": "boom"})
    );
}

#[test]
fn a_completed_operation_keeps_no_error_or_usage() {
    let body = encoded(&json!({"name": OPERATION, "done": true}));
    let video = status_response(&body, Some(PROVIDER), 7, &utc()).expect("the status converts");
    assert_eq!(video.status, "completed");
    assert_eq!(video.created_at, Some(7));
    assert_eq!(video.error, None);
    assert_eq!(video.usage, None);
}

#[test]
fn an_empty_operation_is_processing_with_the_current_time() {
    let video = status_response(b"{}", None, 1_700_000_000, &utc()).expect("the status converts");
    assert_eq!(video.status, "processing");
    assert_eq!(video.id, "");
    assert_eq!(video.model, None);
    assert_eq!(video.created_at, Some(1_700_000_000));
}

#[test]
fn an_empty_error_object_is_kept_without_failing() {
    let body = encoded(&json!({"name": OPERATION, "done": true, "error": {}}));
    let video = status_response(&body, None, 1, &utc()).expect("the status converts");
    assert_eq!(video.status, "completed");
    assert_eq!(
        serde_json::to_value(&video.error).expect("the error converts"),
        json!({})
    );
}

#[test]
fn a_null_error_is_no_error() {
    let body = encoded(&json!({"name": OPERATION, "done": true, "error": null}));
    let video = status_response(&body, None, 1, &utc()).expect("the status converts");
    assert_eq!(video.status, "completed");
    assert_eq!(video.error, None);
}

#[rstest]
#[case::text(br#"{"error": "boom"}"#.as_slice())]
#[case::list(br#"{"error": [1]}"#.as_slice())]
fn a_non_object_error_is_rejected(#[case] body: &[u8]) {
    assert!(matches!(
        status_response(body, None, 0, &utc()),
        Err(VeoOperationError::Json(_))
    ));
}

#[test]
fn a_null_metadata_is_the_current_second() {
    let body = encoded(&json!({"name": OPERATION, "metadata": null}));
    let video = status_response(&body, None, 42, &utc()).expect("the status converts");
    assert_eq!(video.created_at, Some(42));
}

#[test]
fn a_vertex_create_time_becomes_the_created_at() {
    let body = encoded(&json!({
        "name": OPERATION,
        "metadata": {"createTime": "2024-12-04T21:53:12.120184Z"},
    }));
    let video = status_response(&body, None, 1, &utc()).expect("the status converts");
    assert_eq!(video.created_at, Some(1_733_349_192));
}

#[rstest]
#[case::unparseable(text("not a time"))]
#[case::empty(text(""))]
#[case::null(Json::Null)]
fn an_invalid_create_time_uses_the_current_second(#[case] create_time: Json) {
    let body = encoded(&json!({"metadata": {"createTime": create_time}}));
    let video = status_response(&body, None, 42, &utc()).expect("the status converts");
    assert_eq!(video.created_at, Some(42));
}

#[test]
fn a_non_object_metadata_is_rejected() {
    let body = encoded(&json!({"metadata": "later"}));
    assert!(matches!(
        status_response(&body, None, 0, &utc()),
        Err(VeoOperationError::Json(_))
    ));
}

#[test]
fn create_encodes_the_provider_and_returns_processing() {
    let body = encoded(&json!({"name": OPERATION}));
    let data = request_data(object([
        ("durationSeconds", number(6)),
        ("resolution", text("1080p")),
    ]));
    let video = create_response(MODEL, &body, Some(PROVIDER), Some(&data), &settings(8))
        .expect("the create converts");
    assert_eq!(video.status, "processing");
    assert_eq!(video.model.as_deref(), Some(MODEL));
    assert_eq!(
        video.id,
        encode_video_id_with_provider(OPERATION, PROVIDER, Some(MODEL))
    );
    let value = serde_json::to_value(&video).expect("the video converts");
    assert_eq!(value["object"], json!("video"));
    assert_eq!(value["created_at"], json!(null));
    assert_eq!(
        value["usage"],
        json!({"duration_seconds": 6.0, "video_resolution": "1080p"})
    );
}

#[test]
fn create_without_a_provider_keeps_the_operation_name() {
    let body = encoded(&json!({"name": OPERATION}));
    let video =
        create_response(MODEL, &body, None, None, &settings(8)).expect("the create converts");
    assert_eq!(video.id, OPERATION);
    assert_eq!(
        serde_json::to_value(&video.usage).expect("the usage converts"),
        json!({})
    );
}

#[rstest]
#[case::absent(b"{}".as_slice())]
#[case::null(br#"{"name": null}"#.as_slice())]
fn create_requires_an_operation_name(#[case] body: &[u8]) {
    assert!(matches!(
        create_response(MODEL, body, None, None, &settings(8)),
        Err(VeoOperationError::MissingName)
    ));
    assert_eq!(
        VeoOperationError::MissingName.to_string(),
        "No operation name in Veo response"
    );
}

#[test]
fn edit_reads_the_model_and_the_request_usage() {
    let body = encoded(&json!({"name": OPERATION}));
    let data = request_data(object([("durationSeconds", number(4))]));
    let video =
        edit_response(&body, Some(PROVIDER), Some(&data), &settings(8)).expect("the edit converts");
    assert_eq!(video.status, "processing");
    assert_eq!(video.model.as_deref(), Some(MODEL));
    assert_eq!(
        video.id,
        encode_video_id_with_provider(OPERATION, PROVIDER, Some(MODEL))
    );
    assert_eq!(
        serde_json::to_value(&video.usage).expect("the usage converts"),
        json!({"duration_seconds": 4.0})
    );
}

#[test]
fn edit_without_a_model_keeps_an_empty_model() {
    let body = encoded(&json!({"name": "operations/op-1"}));
    let video = edit_response(&body, None, None, &settings(8)).expect("the edit converts");
    assert_eq!(video.model.as_deref(), Some(""));
    assert_eq!(video.id, "operations/op-1");
}

#[rstest]
#[case::absent(b"{}".as_slice())]
#[case::null(br#"{"name": null}"#.as_slice())]
fn edit_requires_an_operation_name(#[case] body: &[u8]) {
    assert!(matches!(
        edit_response(body, None, None, &settings(8)),
        Err(VeoOperationError::MissingEditName)
    ));
    assert_eq!(
        VeoOperationError::MissingEditName.to_string(),
        "No operation name in Veo edit response"
    );
}
