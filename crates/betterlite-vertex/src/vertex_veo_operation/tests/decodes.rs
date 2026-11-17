use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{encoded, number, object, text};
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::vertex_veo_operation::decode::decode_vertex_veo_operation;
use crate::vertex_veo_operation::error::VeoOperationError;

#[test]
fn an_operation_reads_its_name_done_metadata_response_and_error() {
    let body = object([
        ("name", text("projects/p/operations/1")),
        ("done", Json::Bool(true)),
        (
            "metadata",
            Json::Object(object([(
                "createTime",
                text("2024-12-04T21:53:12.120184Z"),
            )])),
        ),
        (
            "response",
            Json::Object(object([(
                "videos",
                Json::List(vec![Json::Object(object([
                    ("bytesBase64Encoded", text("QUJD")),
                    ("gcsUri", text("gs://b/v.mp4")),
                ]))]),
            )])),
        ),
        ("error", Json::Object(object([("code", number(13))]))),
    ]);
    let operation = decode_vertex_veo_operation(&encoded(&body)).expect("the operation decodes");
    assert_eq!(operation.name, "projects/p/operations/1");
    assert!(operation.done);
    assert_eq!(
        operation.metadata.create_time.as_deref(),
        Some("2024-12-04T21:53:12.120184Z")
    );
    assert_eq!(operation.response.videos.len(), 1);
    assert_eq!(operation.response.videos[0].bytes_base64_encoded, "QUJD");
    assert_eq!(
        serde_json::to_value(&operation.error).expect("the error converts"),
        serde_json::to_value(Json::Object(object([("code", number(13))])))
            .expect("the expected error converts")
    );
}

#[test]
fn a_null_or_absent_key_takes_its_default() {
    let operation = decode_vertex_veo_operation(
        br#"{"name": null, "done": null, "metadata": null, "response": null}"#,
    )
    .expect("nulls decode");
    assert_eq!(operation.name, "");
    assert!(!operation.done);
    assert_eq!(operation.metadata.create_time, None);
    assert!(operation.response.videos.is_empty());
    assert_eq!(operation.error, None);
}

#[test]
fn a_null_nested_key_takes_its_default() {
    let operation = decode_vertex_veo_operation(
        br#"{"metadata": {"createTime": null}, "response": {"videos": null}}"#,
    )
    .expect("nested nulls decode");
    assert_eq!(operation.metadata.create_time, None);
    assert!(operation.response.videos.is_empty());
    let video =
        decode_vertex_veo_operation(br#"{"response": {"videos": [{"bytesBase64Encoded": null}]}}"#)
            .expect("a null video field decodes");
    assert_eq!(video.response.videos[0].bytes_base64_encoded, "");
}

#[rstest]
#[case::name(br#"{"name": 5}"#.as_slice())]
#[case::done(br#"{"done": "yes"}"#.as_slice())]
#[case::metadata(br#"{"metadata": "later"}"#.as_slice())]
#[case::create_time(br#"{"metadata": {"createTime": 5}}"#.as_slice())]
#[case::response(br#"{"response": "later"}"#.as_slice())]
#[case::videos(br#"{"response": {"videos": "later"}}"#.as_slice())]
#[case::video(br#"{"response": {"videos": [5]}}"#.as_slice())]
#[case::video_bytes(br#"{"response": {"videos": [{"bytesBase64Encoded": 5}]}}"#.as_slice())]
#[case::error(br#"{"error": "boom"}"#.as_slice())]
fn a_value_of_the_wrong_type_is_rejected(#[case] body: &[u8]) {
    assert!(matches!(
        decode_vertex_veo_operation(body),
        Err(VeoOperationError::Json(_))
    ));
}

#[test]
fn a_body_that_is_not_json_is_rejected() {
    assert!(matches!(
        decode_vertex_veo_operation(b"not json"),
        Err(VeoOperationError::Json(_))
    ));
}

#[test]
fn a_body_over_the_response_cap_is_rejected() {
    let body = vec![b' '; MAX_RESPONSE_BYTES + 1];
    assert!(matches!(
        decode_vertex_veo_operation(&body),
        Err(VeoOperationError::ResponseTooLarge { .. })
    ));
}

#[test]
fn a_body_at_the_response_cap_is_read_as_json() {
    let body = vec![b' '; MAX_RESPONSE_BYTES];
    assert!(matches!(
        decode_vertex_veo_operation(&body),
        Err(VeoOperationError::Json(_))
    ));
}
