use betterlite_prompts::arguments::json::Json;
use serde_json::json;

use super::{encoded, number, object, text};
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::vertex_veo_operation::decode::decode_vertex_veo_operation;
use crate::vertex_veo_operation::error::VeoOperationError;

#[test]
fn an_operation_reads_its_name_done_metadata_and_error() {
    let operation = decode_vertex_veo_operation(&encoded(&json!({
        "name": "projects/p/operations/1",
        "done": true,
        "metadata": {"createTime": "2024-12-04T21:53:12.120184Z"},
        "error": {"code": 13},
    })))
    .expect("the operation decodes");
    assert_eq!(operation.name, "projects/p/operations/1");
    assert!(operation.done);
    assert_eq!(
        operation.metadata,
        Some(Json::Object(object([(
            "createTime",
            text("2024-12-04T21:53:12.120184Z")
        )])))
    );
    assert_eq!(
        operation.error,
        Some(Json::Object(object([("code", number(13))])))
    );
}

#[test]
fn a_null_or_absent_key_takes_its_default() {
    let operation =
        decode_vertex_veo_operation(br#"{"name": null, "done": null, "metadata": null}"#)
            .expect("nulls decode");
    assert_eq!(operation.name, "");
    assert!(!operation.done);
    assert_eq!(operation.metadata, None);
    assert_eq!(operation.response, None);
    assert_eq!(operation.error, None);
}

#[test]
fn a_non_string_name_is_rejected() {
    assert!(decode_vertex_veo_operation(br#"{"name": 5}"#).is_err());
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
