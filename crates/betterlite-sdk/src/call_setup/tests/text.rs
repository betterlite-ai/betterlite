use std::path::PathBuf;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::moment;
use super::support::request;
use super::support::value;
use crate::call_setup::text::OcrFile;
use crate::call_setup::text::debug_text;
use crate::call_setup::text::get_logging_id;
use crate::call_setup::text::ocr_document_summary;

#[test]
fn ocr_summary_defaults_a_non_mapping_document() {
    assert_eq!(ocr_document_summary(None, None), "default-message-value");
    assert_eq!(
        ocr_document_summary(Some(&Json::Text(String::from("x"))), None),
        "default-message-value"
    );
}

#[test]
fn ocr_summary_shortens_a_data_url() {
    assert_eq!(
        ocr_document_summary(
            Some(&value(r#"{"document_url":"data:image/png;base64,abcd"}"#)),
            None,
        ),
        "data:image/png;base64 (4 chars)"
    );
}

#[test]
fn ocr_summary_keeps_a_plain_location() {
    assert_eq!(
        ocr_document_summary(
            Some(&value(r#"{"image_url":"https://example.com/a.png"}"#)),
            None,
        ),
        "https://example.com/a.png"
    );
}

#[test]
fn ocr_summary_keeps_a_plain_location_with_a_comma() {
    assert_eq!(
        ocr_document_summary(
            Some(&value(r#"{"document_url":"https://example.com/a,b.png"}"#)),
            None,
        ),
        "https://example.com/a,b.png"
    );
}

#[test]
fn ocr_summary_prefers_the_document_url_over_a_null_image_url() {
    assert_eq!(
        ocr_document_summary(
            Some(&value(
                r#"{"document_url":"https://example.com/a.png","image_url":"https://example.com/b.png"}"#,
            )),
            None,
        ),
        "https://example.com/a.png"
    );
}

#[test]
fn ocr_summary_names_a_file_kind() {
    assert_eq!(
        ocr_document_summary(Some(&value(r#"{"mime_type":"application/pdf"}"#)), None),
        "file (application/pdf)"
    );
    assert_eq!(ocr_document_summary(Some(&value("{}")), None), "file");
}

#[test]
fn ocr_summary_names_a_path_file() {
    let file = OcrFile::Path(PathBuf::from("docs/report.pdf"));

    assert_eq!(
        ocr_document_summary(
            Some(&value(r#"{"mime_type":"application/pdf"}"#)),
            Some(&file)
        ),
        "file (application/pdf) report.pdf"
    );
}

#[test]
fn ocr_summary_counts_file_bytes() {
    let file = OcrFile::Bytes(vec![0_u8; 7]);

    assert_eq!(
        ocr_document_summary(Some(&value("{}")), Some(&file)),
        "file 7 bytes"
    );
}

#[test]
fn ocr_summary_names_a_named_file() {
    let file = OcrFile::Named(String::from("nested/report.pdf"));

    assert_eq!(
        ocr_document_summary(Some(&value("{}")), Some(&file)),
        "file report.pdf"
    );
}

#[test]
fn logging_id_formats_the_start_time_and_response_id() {
    let response = value(r#"{"id":"resp-1"}"#);

    assert_eq!(
        get_logging_id(moment(), &response),
        Some(String::from("time-00-00-00-000000_resp-1"))
    );
}

#[rstest]
#[case::missing(r#"{"object":"response"}"#)]
#[case::number(r#"{"id":7}"#)]
#[case::not_an_object(r#""resp-1""#)]
fn logging_id_needs_a_string_id(#[case] source: &str) {
    assert_eq!(get_logging_id(moment(), &value(source)), None);
}

#[test]
fn debug_text_is_absent_when_debugging_is_off() {
    let text = debug_text("completion", &request(r#"{"model":"gpt-4o"}"#), false)
        .expect("the request encodes");

    assert_eq!(text, None);
}

#[rstest]
#[case::completion("completion", r#"{"acompletion":true}"#)]
#[case::embedding("embedding", r#"{"aembedding":true}"#)]
#[case::image("img_generation", r#"{"aimg_generation":true}"#)]
fn debug_text_skips_the_inner_call_of_an_async_pair(#[case] name: &str, #[case] extra: &str) {
    let text = debug_text(name, &request(extra), true).expect("the request encodes");

    assert_eq!(text, None);
}

#[test]
fn debug_text_keeps_an_outer_async_call() {
    let text = debug_text(
        "acompletion",
        &request(r#"{"acompletion":true,"model":"gpt-4o"}"#),
        true,
    )
    .expect("the request encodes");

    assert!(
        text.expect("debugging is on")
            .contains("litellm.acompletion(")
    );
}

#[test]
fn debug_text_renders_json_arguments() {
    let text = debug_text(
        "completion",
        &request(r#"{"model":"gpt-4o","temperature":0.5}"#),
        true,
    )
    .expect("the request encodes")
    .expect("debugging is on");

    assert_eq!(
        text,
        "\n\u{1b}[92mRequest to litellm:\u{1b}[0m\n\u{1b}[92mlitellm.completion(model=\"gpt-4o\", temperature=0.5)\u{1b}[0m\n"
    );
}

#[test]
fn debug_text_redacts_credentials() {
    let text = debug_text(
        "completion",
        &request(r#"{"model":"gpt-4o","api_key":"sk-super-secret-value"}"#),
        true,
    )
    .expect("the request encodes")
    .expect("debugging is on");

    assert!(text.contains("api_key=\"REDACTED\""));
    assert!(!text.contains("sk-super-secret-value"));
}

#[test]
fn debug_text_renders_an_empty_call() {
    let text = debug_text("completion", &request("{}"), true)
        .expect("the request encodes")
        .expect("debugging is on");

    assert!(text.contains("litellm.completion()"));
}
