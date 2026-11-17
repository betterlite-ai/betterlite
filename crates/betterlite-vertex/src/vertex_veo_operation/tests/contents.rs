use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{encoded, number, object, text};
use crate::vertex_veo_operation::content::content_response;
use crate::vertex_veo_operation::error::VeoOperationError;

#[test]
fn content_decodes_the_first_videos_bytes() {
    let body = object([
        ("done", Json::Bool(true)),
        (
            "response",
            Json::Object(object([(
                "videos",
                Json::List(vec![
                    Json::Object(object([
                        ("bytesBase64Encoded", text("QUJD")),
                        ("mimeType", text("video/mp4")),
                    ])),
                    Json::Object(object([("bytesBase64Encoded", text("REVG"))])),
                ]),
            )])),
        ),
    ]);
    assert_eq!(
        content_response(&encoded(&body)).expect("the first video decodes"),
        b"ABC"
    );
}

#[test]
fn ignored_characters_decode_to_the_rest_of_the_video() {
    let body = object([
        ("done", Json::Bool(true)),
        (
            "response",
            Json::Object(object([(
                "videos",
                Json::List(vec![Json::Object(object([(
                    "bytesBase64Encoded",
                    text("QU JD\n"),
                )]))]),
            )])),
        ),
    ]);
    assert_eq!(
        content_response(&encoded(&body)).expect("spaces and newlines are ignored"),
        b"ABC"
    );
}

#[test]
fn a_gcs_uri_is_never_downloaded() {
    let body = object([
        ("done", Json::Bool(true)),
        (
            "response",
            Json::Object(object([(
                "videos",
                Json::List(vec![Json::Object(object([(
                    "gcsUri",
                    text("gs://bucket/video.mp4"),
                )]))]),
            )])),
        ),
    ]);
    assert!(matches!(
        content_response(&encoded(&body)),
        Err(VeoOperationError::NoBase64)
    ));
}

#[test]
fn an_unfinished_operation_has_no_content() {
    let body = object([
        ("done", Json::Bool(false)),
        (
            "response",
            Json::Object(object([(
                "videos",
                Json::List(vec![Json::Object(object([(
                    "bytesBase64Encoded",
                    text("QUJD"),
                )]))]),
            )])),
        ),
    ]);
    assert!(matches!(
        content_response(&encoded(&body)),
        Err(VeoOperationError::NotComplete)
    ));
}

#[test]
fn a_completed_operation_without_videos_has_no_content() {
    let body = object([
        ("done", Json::Bool(true)),
        ("response", Json::Object(object([]))),
    ]);
    assert!(matches!(
        content_response(&encoded(&body)),
        Err(VeoOperationError::NoVideos)
    ));
}

#[rstest]
#[case::null_response(br#"{"done": true, "response": null}"#.as_slice())]
#[case::null_videos(br#"{"done": true, "response": {"videos": null}}"#.as_slice())]
fn a_null_content_block_has_no_videos(#[case] body: &[u8]) {
    assert!(matches!(
        content_response(body),
        Err(VeoOperationError::NoVideos)
    ));
}

#[rstest]
#[case::text_response(br#"{"done": true, "response": "later"}"#.as_slice())]
#[case::text_videos(br#"{"done": true, "response": {"videos": "later"}}"#.as_slice())]
#[case::first_video_not_an_object(br#"{"done": true, "response": {"videos": [5]}}"#.as_slice())]
fn a_malformed_content_block_is_rejected(#[case] body: &[u8]) {
    assert!(matches!(
        content_response(body),
        Err(VeoOperationError::Json(_))
    ));
}

#[test]
fn malformed_base64_is_rejected() {
    let body = object([
        ("done", Json::Bool(true)),
        (
            "response",
            Json::Object(object([(
                "videos",
                Json::List(vec![Json::Object(object([(
                    "bytesBase64Encoded",
                    text("QUJ"),
                )]))]),
            )])),
        ),
    ]);
    assert!(matches!(
        content_response(&encoded(&body)),
        Err(VeoOperationError::VideoBase64(_))
    ));
}

#[test]
fn a_non_string_video_field_is_rejected() {
    let body = object([
        ("done", Json::Bool(true)),
        (
            "response",
            Json::Object(object([(
                "videos",
                Json::List(vec![Json::Object(object([(
                    "bytesBase64Encoded",
                    number(5),
                )]))]),
            )])),
        ),
    ]);
    assert!(matches!(
        content_response(&encoded(&body)),
        Err(VeoOperationError::Json(_))
    ));
}

// transform_video_content_response: the three ValueErrors LiteLLM raises keep their exact text
#[test]
fn content_failures_keep_their_messages() {
    assert_eq!(
        VeoOperationError::NotComplete.to_string(),
        "Video generation is not complete yet. Please check status with video_status() before downloading."
    );
    assert_eq!(
        VeoOperationError::NoVideos.to_string(),
        "No video data found in completed operation"
    );
    assert_eq!(
        VeoOperationError::NoBase64.to_string(),
        "No base64 encoded video data found"
    );
}
