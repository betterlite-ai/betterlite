use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{
    MODEL, OPERATION, PROVIDER, context, decimal, encoded, number, object, request_data, settings,
    text, utc,
};
use crate::vertex_veo_operation::error::VeoOperationError;
use crate::vertex_veo_operation::response::{create_response, edit_response, status_response};

#[test]
fn a_failed_operation_beats_done() {
    let body = object([
        ("name", text(OPERATION)),
        ("done", Json::Bool(true)),
        (
            "error",
            Json::Object(object([("code", number(13)), ("message", text("boom"))])),
        ),
    ]);
    let video =
        status_response(&encoded(&body), Some(PROVIDER), 7, &utc()).expect("the status converts");
    assert_eq!(video.status, "failed");
    assert_eq!(
        video.id,
        encode_video_id_with_provider(OPERATION, PROVIDER, Some(MODEL))
    );
    assert_eq!(video.model.as_deref(), Some(MODEL));
    assert_eq!(
        serde_json::to_value(&video.error).expect("the error converts"),
        serde_json::to_value(Json::Object(object([
            ("code", number(13)),
            ("message", text("boom")),
        ])))
        .expect("the expected error converts")
    );
}

#[test]
fn a_completed_operation_keeps_no_error_or_usage() {
    let body = object([("name", text(OPERATION)), ("done", Json::Bool(true))]);
    let video =
        status_response(&encoded(&body), Some(PROVIDER), 7, &utc()).expect("the status converts");
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
    let body = object([
        ("name", text(OPERATION)),
        ("done", Json::Bool(true)),
        ("error", Json::Object(object([]))),
    ]);
    let video = status_response(&encoded(&body), None, 1, &utc()).expect("the status converts");
    assert_eq!(video.status, "completed");
    assert_eq!(
        serde_json::to_value(&video.error).expect("the error converts"),
        serde_json::to_value(Json::Object(object([]))).expect("the expected error converts")
    );
}

#[test]
fn a_null_error_is_no_error() {
    let body = object([
        ("name", text(OPERATION)),
        ("done", Json::Bool(true)),
        ("error", Json::Null),
    ]);
    let video = status_response(&encoded(&body), None, 1, &utc()).expect("the status converts");
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
    let body = object([("name", text(OPERATION)), ("metadata", Json::Null)]);
    let video = status_response(&encoded(&body), None, 42, &utc()).expect("the status converts");
    assert_eq!(video.created_at, Some(42));
}

#[test]
fn a_vertex_create_time_becomes_the_created_at() {
    let body = object([
        ("name", text(OPERATION)),
        (
            "metadata",
            Json::Object(object([(
                "createTime",
                text("2024-12-04T21:53:12.120184Z"),
            )])),
        ),
    ]);
    let video = status_response(&encoded(&body), None, 1, &utc()).expect("the status converts");
    assert_eq!(video.created_at, Some(1_733_349_192));
}

#[rstest]
#[case::unparsable(text("not a time"))]
#[case::empty(text(""))]
#[case::null(Json::Null)]
fn an_invalid_create_time_uses_the_current_second(#[case] create_time: Json) {
    let body = object([(
        "metadata",
        Json::Object(object([("createTime", create_time)])),
    )]);
    let video = status_response(&encoded(&body), None, 42, &utc()).expect("the status converts");
    assert_eq!(video.created_at, Some(42));
}

#[test]
fn a_non_object_metadata_is_rejected() {
    let body = object([("metadata", text("later"))]);
    assert!(matches!(
        status_response(&encoded(&body), None, 0, &utc()),
        Err(VeoOperationError::Json(_))
    ));
}

#[test]
fn create_encodes_the_provider_and_returns_processing() {
    let body = object([("name", text(OPERATION))]);
    let data = request_data(object([
        ("durationSeconds", number(6)),
        ("resolution", text("1080p")),
    ]));
    let video = create_response(
        MODEL,
        &encoded(&body),
        &context(Some(PROVIDER), Some(&data), &settings(8)),
    )
    .expect("the create converts");
    assert_eq!(video.status, "processing");
    assert_eq!(video.model.as_deref(), Some(MODEL));
    assert_eq!(
        video.id,
        encode_video_id_with_provider(OPERATION, PROVIDER, Some(MODEL))
    );
    let value = serde_json::to_value(&video).expect("the video converts");
    assert_eq!(
        value["object"],
        serde_json::to_value(text("video")).expect("the object name converts")
    );
    assert!(value["created_at"].is_null());
    assert_eq!(
        value["usage"],
        serde_json::to_value(Json::Object(object([
            ("duration_seconds", decimal(6.0)),
            ("video_resolution", text("1080p")),
        ])))
        .expect("the expected usage converts")
    );
}

#[test]
fn create_without_a_provider_keeps_the_operation_name() {
    let body = object([("name", text(OPERATION))]);
    let video = create_response(MODEL, &encoded(&body), &context(None, None, &settings(8)))
        .expect("the create converts");
    assert_eq!(video.id, OPERATION);
    assert_eq!(
        serde_json::to_value(&video.usage).expect("the usage converts"),
        serde_json::to_value(Json::Object(object([]))).expect("the expected usage converts")
    );
}

// create and edit refuse the same absent or null name, each with its own message
#[derive(Clone, Copy, Debug)]
enum Start {
    Create,
    Edit,
}

#[rstest]
#[case::create_absent(Start::Create, b"{}".as_slice())]
#[case::create_null(Start::Create, br#"{"name": null}"#.as_slice())]
#[case::edit_absent(Start::Edit, b"{}".as_slice())]
#[case::edit_null(Start::Edit, br#"{"name": null}"#.as_slice())]
fn a_missing_operation_name_is_rejected(#[case] start: Start, #[case] body: &[u8]) {
    let error = match start {
        Start::Create => create_response(MODEL, body, &context(None, None, &settings(8))),
        Start::Edit => edit_response(body, &context(None, None, &settings(8))),
    }
    .expect_err("the operation name is missing");
    let message = match start {
        Start::Create => "No operation name in Veo response",
        Start::Edit => "No operation name in Veo edit response",
    };
    assert_eq!(error.to_string(), message);
}

#[test]
fn edit_reads_the_model_and_the_request_usage() {
    let body = object([("name", text(OPERATION))]);
    let data = request_data(object([("durationSeconds", number(4))]));
    let video = edit_response(
        &encoded(&body),
        &context(Some(PROVIDER), Some(&data), &settings(8)),
    )
    .expect("the edit converts");
    assert_eq!(video.status, "processing");
    assert_eq!(video.model.as_deref(), Some(MODEL));
    assert_eq!(
        video.id,
        encode_video_id_with_provider(OPERATION, PROVIDER, Some(MODEL))
    );
    assert_eq!(
        serde_json::to_value(&video.usage).expect("the usage converts"),
        serde_json::to_value(Json::Object(object([("duration_seconds", decimal(4.0))])))
            .expect("the expected usage converts")
    );
}

#[test]
fn edit_without_a_model_keeps_an_empty_model() {
    let body = object([("name", text("operations/op-1"))]);
    let video = edit_response(&encoded(&body), &context(None, None, &settings(8)))
        .expect("the edit converts");
    assert_eq!(video.model.as_deref(), Some(""));
    assert_eq!(video.id, "operations/op-1");
}
