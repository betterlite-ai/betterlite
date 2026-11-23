#![cfg(test)]

use betterlite_ocr::json::Json;

use super::SessionCore;
use crate::search_ocr::convert::OcrCall;

#[test]
fn consumed_params_keeps_only_the_names_the_provider_reads() {
    let session = SessionCore::build(r#"{"environment": {}}"#).expect("the session builds");
    let call: OcrCall = serde_json::from_str(
        r#"{
            "model": "mistral-ocr-latest",
            "document": {"type": "document_url", "document_url": "https://example.test/a.pdf"},
            "optional_params": {"req_format": "openai", "unknown_option": true}
        }"#,
    )
    .expect("the ocr call parses");
    let kept = session
        .consumed_params(&call, "mistral-ocr-latest", "mistral")
        .expect("the provider config resolves");
    assert_eq!(
        kept.get("req_format"),
        Some(&Json::Text(String::from("openai")))
    );
    assert!(!kept.contains_key("unknown_option"));
}
