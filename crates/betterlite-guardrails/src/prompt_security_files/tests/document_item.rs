#![cfg(test)]

use std::collections::BTreeMap;

use reqwest::StatusCode;
use rstest::rstest;

use super::shared::{
    DONE_ALLOW, UPLOADED, body_text, data_url, done, object, sanitizer, text, vendor,
};
use crate::prompt_security_files::document::process_document_item;
use crate::prompt_security_files::error::FileSanitizationError;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

const REDACTED_BASE64: &str = "UkVEQUNURUQ=";

fn nested(key: &str, fields: &[(&str, GuardrailValue)]) -> BTreeMap<String, GuardrailValue> {
    object(&[
        ("type", text(key)),
        (key, GuardrailValue::Object(object(fields))),
    ])
}

#[rstest]
#[case::default_is_pdf(None, "document.pdf")]
#[case::pdf(Some("application/pdf"), "document.pdf")]
#[case::word(Some("application/msword"), "document.docx")]
#[case::docx(
    Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document.docx"),
    "document.docx"
)]
#[case::excel(Some("application/vnd.ms-excel"), "document.xlsx")]
#[case::xlsx(Some("application/x-xlsx"), "document.xlsx")]
#[case::other(Some("text/plain"), "document.plain")]
#[tokio::test]
async fn document_item_infers_the_filename_from_the_data_field_mime_type(
    #[case] mime_type: Option<&str>,
    #[case] filename: &str,
) {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let mut fields = vec![("data", text("aGVsbG8="))];
    if let Some(declared) = mime_type {
        fields.push(("mime_type", text(declared)));
    }
    let item = nested("document", &fields);
    let processed = process_document_item(&sanitizer(&server.base), item.clone(), Some("alias"))
        .await
        .expect("allowed");
    assert_eq!(processed, item);
    let uploads = server.requests("POST");
    let body = body_text(uploads.first().expect("upload"));
    assert!(body.contains(&format!(r#"filename="{filename}""#)));
    assert!(body.contains("hello"));
}

#[tokio::test]
async fn document_item_reads_the_mime_type_from_a_data_url() {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let item = nested(
        "file",
        &[("url", text(&data_url("application/msword", "hello")))],
    );
    process_document_item(&sanitizer(&server.base), item, None)
        .await
        .expect("allowed");
    let uploads = server.requests("POST");
    assert!(body_text(uploads.first().expect("upload")).contains(r#"filename="document.docx""#));
}

#[rstest]
#[case::document_url("document", "url")]
#[case::file_url("file", "url")]
#[tokio::test]
async fn document_item_replaces_a_data_url_keeping_its_header(
    #[case] key: &str,
    #[case] field: &str,
) {
    let server = vendor(UPLOADED, &[done("modify", "REDACTED")]).await;
    let mut guardrail = sanitizer(&server.base);
    guardrail.settings.block_on_file_modify = false;
    let item = nested(key, &[(field, text(&data_url("application/pdf", "hello")))]);
    let processed = process_document_item(&guardrail, item, None)
        .await
        .expect("modified");
    let expected = nested(
        key,
        &[(
            field,
            text(&format!("data:application/pdf;base64,{REDACTED_BASE64}")),
        )],
    );
    assert_eq!(processed, expected);
}

#[rstest]
#[case::document("document")]
#[case::file("file")]
#[tokio::test]
async fn document_item_replaces_the_data_field(#[case] key: &str) {
    let server = vendor(UPLOADED, &[done("modify", "REDACTED")]).await;
    let mut guardrail = sanitizer(&server.base);
    guardrail.settings.block_on_file_modify = false;
    let item = nested(
        key,
        &[
            ("data", text("aGVsbG8=")),
            ("mime_type", text("application/pdf")),
        ],
    );
    let processed = process_document_item(&guardrail, item, None)
        .await
        .expect("modified");
    let expected = nested(
        key,
        &[
            ("data", text(REDACTED_BASE64)),
            ("mime_type", text("application/pdf")),
        ],
    );
    assert_eq!(processed, expected);
}

#[tokio::test]
async fn document_item_reads_and_replaces_fields_on_the_item_itself() {
    let server = vendor(UPLOADED, &[done("modify", "REDACTED")]).await;
    let mut guardrail = sanitizer(&server.base);
    guardrail.settings.block_on_file_modify = false;
    let item = object(&[("type", text("document")), ("data", text("aGVsbG8="))]);
    let processed = process_document_item(&guardrail, item, None)
        .await
        .expect("modified");
    assert_eq!(processed["data"], text(REDACTED_BASE64));
}

#[tokio::test]
async fn document_item_sanitizes_a_string_valued_document_but_cannot_replace_it() {
    let server = vendor(UPLOADED, &[done("modify", "REDACTED")]).await;
    let mut guardrail = sanitizer(&server.base);
    guardrail.settings.block_on_file_modify = false;
    let item = object(&[
        ("type", text("file")),
        ("file", text(&data_url("application/pdf", "hello"))),
    ]);
    let processed = process_document_item(&guardrail, item.clone(), None)
        .await
        .expect("kept");
    assert_eq!(processed, item);
    assert_eq!(server.requests("POST").len(), 1);
}

#[rstest]
#[case::no_fields(nested("file", &[("filename", text("a.pdf"))]))]
#[case::remote_url(nested("document", &[("url", text("https://example.test/a.pdf"))]))]
#[case::empty_data(nested("document", &[("data", text(""))]))]
#[tokio::test]
async fn document_item_leaves_items_without_inline_content_alone(
    #[case] item: BTreeMap<String, GuardrailValue>,
) {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let processed = process_document_item(&sanitizer(&server.base), item.clone(), None)
        .await
        .expect("untouched");
    assert_eq!(processed, item);
    assert!(server.requests("POST").is_empty());
}

#[rstest]
#[case::block("block")]
#[case::modify("modify")]
#[tokio::test]
async fn document_item_raises_a_400_for_a_blocked_document(#[case] action: &str) {
    let server = vendor(UPLOADED, &[done(action, "REDACTED")]).await;
    let item = nested("document", &[("data", text("aGVsbG8="))]);
    let error = process_document_item(&sanitizer(&server.base), item, None)
        .await
        .expect_err("blocked");
    assert!(matches!(
        &error,
        FileSanitizationError::Blocked {
            resource: "Document",
            ..
        }
    ));
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
}
