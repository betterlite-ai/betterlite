use betterlite_http::error::HttpError;

use crate::openai_video_identifiers::ids::encode_video_id_with_provider;
use crate::videos::content::content_request;
use crate::videos::error::VideoError;

use super::support::{API_BASE, MODEL, SLASH_BASE};

const VARIANTS: [(&str, &str); 5] = [
    ("", "?variant="),
    ("thumbnail&extra=1", "?variant=thumbnail%26extra%3D1"),
    ("h\u{e9}llo", "?variant=h%C3%A9llo"),
    ("a b/c", "?variant=a%20b%2Fc"),
    ("a-b.c_d~e", "?variant=a-b.c_d~e"),
];

pub(super) fn binary_content_case() {
    let encoded = encode_video_id_with_provider("vid-6", "openai", Some(MODEL));
    let (url, params) = content_request(&encoded, SLASH_BASE, None).unwrap();
    assert_eq!(url, "https://mock.example/v1/videos/vid-6/content");
    assert!(params.is_empty());
}

pub(super) fn content_variant_case() {
    let encoded = encode_video_id_with_provider("vid/10", "openai", Some(MODEL));
    let (url, params) = content_request(&encoded, SLASH_BASE, None).unwrap();
    assert_eq!(url, "https://mock.example/v1/videos/vid%2F10/content");
    assert!(params.is_empty());
    for (variant, query) in VARIANTS {
        let (variant_url, _) = content_request(&encoded, API_BASE, Some(variant)).unwrap();
        assert_eq!(
            variant_url,
            format!("https://mock.example/v1/videos/vid%2F10/content{query}")
        );
    }
    let missing = content_request("", API_BASE, None);
    assert!(matches!(
        missing,
        Err(VideoError::Path {
            source: HttpError::Required { .. }
        })
    ));
    let dots = content_request("..", API_BASE, None);
    assert!(matches!(
        dots,
        Err(VideoError::Path {
            source: HttpError::DotSegment { .. }
        })
    ));
}
