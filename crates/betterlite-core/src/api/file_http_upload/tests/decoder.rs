use std::path::Path;

use betterlite_openai::api::file_type_records::UploadUrlLocation;
use http::{HeaderMap, HeaderValue};
use rstest::rstest;

use crate::api::api_boundaries::ApiLimits;
use crate::api::file_http_upload::decoder::{decode_file_upload_location, upload_url_from_headers};

use super::{UPLOAD_URL, text};

#[test]
fn decode_file_upload_location_reads_only_the_body_location() {
    let limits = ApiLimits::default();
    let headers = {
        let mut headers = HeaderMap::new();
        headers.insert("X-Goog-Upload-URL", HeaderValue::from_static(UPLOAD_URL));
        headers
    };
    assert_eq!(
        upload_url_from_headers(&headers),
        Some(String::from(UPLOAD_URL))
    );
    // the header lookup is case-insensitive and an empty value counts as absent
    let mut empty = HeaderMap::new();
    empty.insert("x-goog-upload-url", HeaderValue::from_static(""));
    assert_eq!(upload_url_from_headers(&empty), None);
    // the headers location never parses the body, even a broken one
    let located = decode_file_upload_location(
        b"not json",
        UploadUrlLocation::Headers,
        "upload_url",
        limits,
    )
    .expect("a headers location never fails");
    assert_eq!(located.url, None);
    assert_eq!(located.data, None);
}

#[rstest]
#[case::truncated("{")]
#[case::empty("")]
#[case::null("null")]
#[case::number("7")]
#[case::string("\"http://a\"")]
#[case::array("[{\"upload_url\":\"http://a\"}]")]
#[case::empty_object("{}")]
#[case::null_value("{\"upload_url\":null}")]
#[case::empty_value("{\"upload_url\":\"\"}")]
#[case::number_value("{\"upload_url\":7}")]
#[case::object_value("{\"upload_url\":{}}")]
#[case::other_key("{\"other\":\"http://a\"}")]
fn decode_file_upload_location_rejects_every_shape_that_names_no_url(#[case] body: &str) {
    let limits = ApiLimits::default();
    let located = decode_file_upload_location(
        body.as_bytes(),
        UploadUrlLocation::Body,
        "upload_url",
        limits,
    )
    .expect("a body that names no URL is not an error");
    assert_eq!(located.url, None, "for {body}");
    assert_eq!(located.data, None);
}

#[test]
fn decode_file_upload_location_reads_the_named_key() {
    let limits = ApiLimits::default();
    let body = br#"{"upload_url":"http://wrong","target":"http://right","kept":"yes"}"#;
    let located = decode_file_upload_location(body, UploadUrlLocation::Body, "target", limits)
        .expect("a located URL");
    assert_eq!(located.url, Some(String::from("http://right")));
    let data = located.data.expect("the parsed body");
    assert_eq!(data.get("kept"), Some(&text("yes")));
}

#[test]
fn decode_file_upload_location_is_bounded() {
    let limits = ApiLimits {
        max_json_bytes: 4,
        max_event_bytes: 1024 * 1024,
        max_items: 64 * 1024,
    };
    let error = decode_file_upload_location(
        b"{\"upload_url\":\"http://a\"}",
        UploadUrlLocation::Body,
        "upload_url",
        limits,
    )
    .expect_err("a body over the limit");
    assert!(error.to_string().contains("the maximum is 4"));
}

#[test]
fn replays_upload_location_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/file_upload_location");
    let replayed = std::fs::read_dir(corpus)
        .expect("the corpus directory")
        .map(|entry| std::fs::read(entry.expect("a corpus entry").path()).expect("a corpus file"))
        .map(|bytes| {
            drop(decode_file_upload_location(
                &bytes,
                UploadUrlLocation::Body,
                "upload_url",
                ApiLimits::default(),
            ));
            drop(upload_url_from_headers(&HeaderMap::new()));
        })
        .count();
    assert!(replayed >= 8);
}
