use std::collections::BTreeMap;

use http::HeaderMap;
use http::header::HeaderValue;

use crate::gemini_http::error::get_error_class;

// transform_video_*_error_class: the provider error carries the response status, message and headers unchanged
#[test]
fn the_provider_error_class_keeps_the_status_message_and_headers() {
    let mut headers = HeaderMap::new();
    headers.insert("x-goog-request-id", HeaderValue::from_static("abc"));
    let error = get_error_class("quota exceeded".to_owned(), 429, &headers);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "quota exceeded");
    assert_eq!(
        error.headers,
        Some(BTreeMap::from([(
            "x-goog-request-id".to_owned(),
            "abc".to_owned()
        )]))
    );
}
