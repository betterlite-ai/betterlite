#![cfg(test)]

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use super::{Input, Op};
use crate::helpers::Helper as _;

#[test]
fn extract_file_metadata_guesses_the_content_type_from_the_filename() {
    let input: Input = serde_json::from_str(
        r#"{"operation":"extract_file_metadata","file_data":{"kind":"tuple2","filename":"photo.png","content":{"kind":"bytes","content_base64":"YWJj"}}}"#,
    )
    .unwrap();
    let value = Op::run(&input).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"filename": "photo.png", "content_type": "image/png"})
    );
}

#[test]
fn extract_file_data_rejects_a_bare_string_body() {
    let input: Input = serde_json::from_str(
        r#"{"operation":"extract_file_data","file_data":{"kind":"direct","content":{"kind":"bare_string","content":"/etc/passwd"}}}"#,
    )
    .unwrap();
    let error = Op::run(&input).unwrap_err();
    assert_eq!(error.class, "ValueError");
}

#[test]
fn infer_content_type_prefers_the_url_extension() {
    let content_base64 = BASE64.encode(b"");
    let input: Input = serde_json::from_str(&format!(
        r#"{{"operation":"infer_content_type_from_url_and_content","url":"https://example.com/document.pdf","content_base64":"{content_base64}","current_content_type":null}}"#
    ))
    .unwrap();
    let value = Op::run(&input).unwrap();
    assert_eq!(value, serde_json::json!("application/pdf"));
}
