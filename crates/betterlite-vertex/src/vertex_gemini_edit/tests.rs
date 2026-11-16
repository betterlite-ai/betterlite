#![cfg(test)]

use std::io::{Cursor, Seek, SeekFrom};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use http::header::{HeaderMap, HeaderValue};
use rstest::rstest;

use super::error::GeminiEditError;
use super::options::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::request::{GeminiEditFile, transform_image_edit_request};
use super::response::{GeminiEditResponse, transform_image_edit_response};
use crate::vertex_gemini_images::error::GeminiImageError;

fn params(source: &str) -> ImageParams {
    serde_json::from_str(source).expect("params source")
}

fn json_value(source: &str) -> Json {
    serde_json::from_str(source).expect("json source")
}

fn reply(status: u16, headers: HeaderMap, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

fn request_body(
    prompt: Option<&str>,
    images: &mut [Option<GeminiEditFile<'_>>],
    optional: &ImageParams,
) -> Json {
    let (body, files) = transform_image_edit_request(prompt, images, optional, &|_| "image/png")
        .expect("the request builds");
    assert!(files.is_empty());
    Json::Object(body)
}

fn encoded(response: &GeminiEditResponse) -> Json {
    serde_json::from_str(&serde_json::to_string(response).expect("the response encodes"))
        .expect("the response is JSON")
}

#[test]
fn only_size_is_advertised() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["size"]);
}

#[rstest]
#[case::square("1024x1024", "1:1")]
#[case::wide("1792x1024", "16:9")]
#[case::tall("1024x1792", "9:16")]
#[case::landscape("1280x896", "4:3")]
#[case::portrait("896x1280", "3:4")]
#[case::unknown("999x999", "1:1")]
fn a_size_maps_to_its_aspect_ratio(#[case] size: &str, #[case] ratio: &str) {
    let mapped =
        map_openai_params(&params(&format!(r#"{{"size":"{size}"}}"#))).expect("a string size maps");
    assert_eq!(
        Json::Object(mapped),
        json_value(&format!(r#"{{"aspectRatio":"{ratio}"}}"#))
    );
}

#[test]
fn an_absent_size_maps_to_nothing_and_a_null_one_to_a_square() {
    assert!(
        map_openai_params(&params("{}"))
            .expect("no size maps")
            .is_empty()
    );
    let mapped = map_openai_params(&params(r#"{"size":null}"#)).expect("a null size maps");
    assert_eq!(Json::Object(mapped), json_value(r#"{"aspectRatio":"1:1"}"#));
}

#[test]
fn a_non_string_size_names_the_field() {
    let error = map_openai_params(&params(r#"{"size":1024}"#)).expect_err("a number is not a size");
    assert!(matches!(
        error,
        GeminiEditError::FieldType { param: "size", .. }
    ));
}

#[test]
fn the_contents_are_one_user_object_with_the_images_before_the_prompt() {
    let mut images = [
        Some(GeminiEditFile::Bytes(b"\x89PNG\r\n\x1a\none")),
        None,
        Some(GeminiEditFile::Bytes(b"GIF89a two")),
    ];
    let body = request_body(Some("make it blue"), &mut images, &params("{}"));
    assert_eq!(
        body,
        json_value(
            r#"{"contents":{"role":"USER","parts":[{"inlineData":{"mimeType":"image/png","data":"iVBORw0KGgpvbmU="}},{"inlineData":{"mimeType":"image/png","data":"R0lGODlhIHR3bw=="}},{"text":"make it blue"}]},"generationConfig":{"response_modalities":["IMAGE"]}}"#
        )
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn an_absent_or_empty_prompt_adds_no_text_part(#[case] prompt: Option<&str>) {
    let mut images = [Some(GeminiEditFile::Bytes(b"\x89PNG"))];
    let body = request_body(prompt, &mut images, &params("{}"));
    assert_eq!(
        body,
        json_value(
            r#"{"contents":{"role":"USER","parts":[{"inlineData":{"mimeType":"image/png","data":"iVBORw=="}}]},"generationConfig":{"response_modalities":["IMAGE"]}}"#
        )
    );
}

#[test]
fn the_mapped_aspect_ratio_lands_in_the_snake_case_image_config() {
    let mut images = [Some(GeminiEditFile::Bytes(b"\x89PNG"))];
    let body = request_body(None, &mut images, &params(r#"{"aspectRatio":"16:9"}"#));
    assert_eq!(
        body,
        json_value(
            r#"{"contents":{"role":"USER","parts":[{"inlineData":{"mimeType":"image/png","data":"iVBORw=="}}]},"generationConfig":{"response_modalities":["IMAGE"],"image_config":{"aspect_ratio":"16:9"}}}"#
        )
    );
}

#[test]
fn a_list_with_no_image_part_is_refused() {
    let error = transform_image_edit_request(None, &mut [], &params("{}"), &|_| "image/png")
        .expect_err("an empty list has no image part");
    assert!(matches!(error, GeminiEditError::MissingImage));
    let mut none = [None];
    let error = transform_image_edit_request(None, &mut none, &params("{}"), &|_| "image/png")
        .expect_err("a list of Nones has no image part");
    assert!(matches!(error, GeminiEditError::MissingImage));
}

#[test]
fn an_unsupported_file_record_is_rejected() {
    let mut images = [Some(GeminiEditFile::Unsupported)];
    let error = transform_image_edit_request(None, &mut images, &params("{}"), &|_| "image/png")
        .expect_err("an unsupported record has no bytes");
    assert!(matches!(error, GeminiEditError::UnsupportedImage));
}

#[test]
fn a_seekable_image_is_read_from_zero_and_its_cursor_restored() {
    let mut content = Cursor::new(b"0123456789abcdef".to_vec());
    content.seek(SeekFrom::Start(7)).expect("the cursor moves");
    let mut images = [Some(GeminiEditFile::Seekable(&mut content))];
    let body = request_body(None, &mut images, &params("{}"));
    assert_eq!(
        body,
        json_value(
            r#"{"contents":{"role":"USER","parts":[{"inlineData":{"mimeType":"image/png","data":"MDEyMzQ1Njc4OWFiY2RlZg=="}}]},"generationConfig":{"response_modalities":["IMAGE"]}}"#
        )
    );
    assert_eq!(content.position(), 7);
}

#[test]
fn the_injected_mime_selector_reads_the_image_content() {
    let mut images = [Some(GeminiEditFile::Bytes(b"GIF89a two"))];
    let (body, files) =
        transform_image_edit_request(None, &mut images, &params("{}"), &|content| {
            if content.starts_with(b"GIF8") {
                "image/gif"
            } else {
                "image/png"
            }
        })
        .expect("the request builds");
    assert!(files.is_empty());
    assert_eq!(
        Json::Object(body),
        json_value(
            r#"{"contents":{"role":"USER","parts":[{"inlineData":{"mimeType":"image/gif","data":"R0lGODlhIHR3bw=="}}]},"generationConfig":{"response_modalities":["IMAGE"]}}"#
        )
    );
}

#[test]
fn truthy_inline_data_only_becomes_the_edit_data() {
    let body = r#"{"candidates":[{"content":{"parts":[{"inlineData":{"mimeType":"image/png","data":"AAA"},"thoughtSignature":"sig"},{"text":"ignored"}]}},{"content":{"parts":[{"inlineData":{"data":"BBB"}}]}},{"finishReason":"STOP"}],"usageMetadata":{"totalTokenCount":9}}"#;
    let response = transform_image_edit_response(&reply(200, HeaderMap::new(), body))
        .expect("the reply decodes");
    assert_eq!(
        encoded(&response),
        json_value(r#"{"data":[{"b64_json":"AAA"},{"b64_json":"BBB"}]}"#)
    );
}

#[rstest]
#[case::empty_string(r#"{"candidates":[{"content":{"parts":[{"inlineData":{"data":""}}]}}]}"#)]
#[case::null_data(r#"{"candidates":[{"content":{"parts":[{"inlineData":{"data":null}}]}}]}"#)]
#[case::empty_inline_data(r#"{"candidates":[{"content":{"parts":[{"inlineData":{}}]}}]}"#)]
#[case::absent_inline_data(r#"{"candidates":[{"content":{"parts":[{"text":"x"}]}}]}"#)]
#[case::no_candidates("{}")]
#[case::null_candidates(r#"{"candidates":null}"#)]
fn empty_inline_data_is_dropped(#[case] body: &str) {
    let response = transform_image_edit_response(&reply(200, HeaderMap::new(), body))
        .expect("the reply decodes");
    assert_eq!(encoded(&response), json_value(r#"{"data":[]}"#));
}

#[test]
fn a_reply_that_is_not_an_object_names_the_shape() {
    let error = transform_image_edit_response(&reply(200, HeaderMap::new(), "[]"))
        .expect_err("a list root is not a candidates document");
    assert!(matches!(
        error,
        GeminiEditError::Reply(GeminiImageError::Shape { .. })
    ));
    assert!(error.upstream_answer().is_none());
}

#[test]
fn a_non_json_reply_keeps_the_upstream_answer() {
    let mut headers = HeaderMap::new();
    headers.insert("x-request-id", HeaderValue::from_static("req-1"));
    let error = transform_image_edit_response(&reply(502, headers, r#"{"candidates": ["#))
        .expect_err("a truncated reply fails");
    let (status, headers) = error
        .upstream_answer()
        .expect("a parse failure keeps the reply answer");
    assert_eq!(status, 502);
    assert_eq!(
        headers.get("x-request-id"),
        Some(&HeaderValue::from_static("req-1"))
    );
}
