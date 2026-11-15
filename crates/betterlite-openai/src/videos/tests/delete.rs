use betterlite_http::error::HttpError;
use serde_json::{Value, json};

use crate::openai_video_identifiers::ids::encode_video_id_with_provider;
use crate::videos::error::{VideoError, get_error_class};
use crate::videos::reply::decode_video;
use crate::videos::segment::path_url;

use super::support::{API_BASE, MODEL, SLASH_BASE, body, headers};

fn delete_url_case() {
    let encoded = encode_video_id_with_provider("vid-5", "openai", Some(MODEL));
    assert_eq!(
        path_url(&encoded, SLASH_BASE, "").unwrap(),
        "https://mock.example/v1/videos/vid-5"
    );
}

fn delete_path_errors_case() {
    let missing = path_url("", API_BASE, "").unwrap_err();
    assert!(matches!(
        &missing,
        VideoError::Path { source: HttpError::Required { field_name } } if field_name == "video_id"
    ));
    assert_eq!(missing.to_string(), "video_id is required");
    let dot = path_url(".", API_BASE, "").unwrap_err();
    assert!(matches!(
        &dot,
        VideoError::Path { source: HttpError::DotSegment { field_name } } if field_name == "video_id"
    ));
    assert_eq!(dot.to_string(), "video_id cannot be a dot path segment");
    let dot_dot = path_url("..", API_BASE, "").unwrap_err();
    assert!(matches!(
        dot_dot,
        VideoError::Path {
            source: HttpError::DotSegment { .. }
        }
    ));
}

fn delete_reply_case() {
    let reply = json!({
        "id": "vid-5",
        "object": "video",
        "status": "deleted",
        "seconds": "4",
        "usage": {"duration_seconds": 99_i64}
    });
    let deleted = decode_video(&body(&reply)).unwrap();
    assert_eq!(deleted.id, "vid-5");
    assert_eq!(deleted.seconds, Some(json!("4")));
    assert_eq!(
        Value::Object(deleted.usage.unwrap()),
        json!({"duration_seconds": 99_i64})
    );
    let list_shape = decode_video(&body(&json!({"data": []})));
    assert!(matches!(list_shape, Err(VideoError::ExpectedVideo)));
}

fn error_class_case() {
    let error = get_error_class("rate limited", 429, headers(&[("retry-after", "1")]));
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "rate limited");
    assert_eq!(
        error.headers,
        Some(vec![("retry-after".to_owned(), "1".to_owned())])
    );
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.request.method, "POST");
    assert_eq!(error.request.url, "https://docs.litellm.ai/docs");
    assert_eq!(error.response.status_code, 429);
    assert_eq!(error.body, None);
    let empty = get_error_class("missing", 400, headers(&[]));
    assert_eq!(empty.headers, Some(Vec::new()));
}

pub(super) fn delete_case() {
    delete_url_case();
    delete_path_errors_case();
    delete_reply_case();
    error_class_case();
}
