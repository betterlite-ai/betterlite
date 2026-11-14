mod execute;

use betterlite_http::proxy::limits::TransportLimits;
use rstest::rstest;

use super::support::json;
use crate::gemini_embedding_call::error::GeminiEmbeddingCallError;
use crate::gemini_embedding_call::http::{
    EMBEDDING_BODY_LIMIT, decode_embedding_http, file_metadata,
};

#[rstest]
#[case::both(
    r#"{"mimeType":"image/png","uri":"https://x/files/a"}"#,
    "image/png",
    "https://x/files/a"
)]
#[case::missing_mime_type(r#"{"uri":"https://x/files/a"}"#, "", "https://x/files/a")]
#[case::missing_uri(r#"{"mimeType":"image/png"}"#, "image/png", "files/a")]
#[case::both_missing("{}", "", "files/a")]
#[case::null_takes_the_default(r#"{"mimeType":null,"uri":null}"#, "", "files/a")]
#[case::empty_text_is_kept(r#"{"mimeType":"","uri":""}"#, "", "")]
#[case::other_fields_are_ignored(r#"{"name":"files/a","state":"ACTIVE"}"#, "", "files/a")]
fn reads_file_metadata(#[case] reply: &str, #[case] mime_type: &str, #[case] uri: &str) {
    let resolved = file_metadata("files/a", &json(reply)).unwrap();
    assert_eq!(resolved.mime_type, mime_type);
    assert_eq!(resolved.uri, uri);
}

#[rstest]
#[case::list("[]")]
#[case::text(r#""files/a""#)]
#[case::null("null")]
fn metadata_must_be_an_object(#[case] reply: &str) {
    let error = file_metadata("files/a", &json(reply)).unwrap_err();
    assert!(matches!(
        &error,
        GeminiEmbeddingCallError::MetadataShape { file } if file == "files/a"
    ));
}

#[rstest]
#[case::mime_number(r#"{"mimeType":1}"#, "mimeType")]
#[case::mime_object(r#"{"mimeType":{}}"#, "mimeType")]
#[case::uri_bool(r#"{"uri":true}"#, "uri")]
#[case::uri_list(r#"{"mimeType":"a","uri":[]}"#, "uri")]
fn metadata_fields_must_be_text(#[case] reply: &str, #[case] field: &str) {
    let error = file_metadata("files/a", &json(reply)).unwrap_err();
    assert!(matches!(
        &error,
        GeminiEmbeddingCallError::MetadataField { file: name, field: named }
            if name == "files/a" && *named == field
    ));
}

#[test]
fn the_body_limit_matches_the_pooled_client() {
    assert_eq!(EMBEDDING_BODY_LIMIT, TransportLimits::DEFAULT.body_bytes);
}

#[test]
fn decodes_json_up_to_the_limit() {
    let mut bytes = vec![b' '; EMBEDDING_BODY_LIMIT - 2];
    bytes.extend_from_slice(b"[]");
    assert_eq!(bytes.len(), EMBEDDING_BODY_LIMIT);
    assert_eq!(decode_embedding_http(&bytes).unwrap(), json("[]"));
}

#[test]
fn refuses_a_body_past_the_limit() {
    let bytes = vec![b' '; EMBEDDING_BODY_LIMIT + 1];
    let error = decode_embedding_http(&bytes).unwrap_err();
    assert!(matches!(
        error,
        GeminiEmbeddingCallError::BodyTooLarge { limit } if limit == EMBEDDING_BODY_LIMIT
    ));
}

#[rstest]
#[case::empty(b"")]
#[case::truncated(b"{\"embedding\":")]
#[case::not_utf8(b"\xff\xfe")]
fn refuses_a_body_that_is_not_json(#[case] bytes: &[u8]) {
    assert!(matches!(
        decode_embedding_http(bytes),
        Err(GeminiEmbeddingCallError::Decode(_))
    ));
}
