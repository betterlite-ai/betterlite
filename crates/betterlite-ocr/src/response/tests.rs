#![cfg(test)]

use indexmap::IndexMap;

use super::{OCRPage, OCRPageDimensions, OCRPageImage, OCRResponse, OCRUsageInfo};
use crate::json::Json;

#[test]
fn ocr_page_dimensions_defaults_to_null_fields() {
    let dimensions: OCRPageDimensions = serde_json::from_str("{}").unwrap();
    assert_eq!(
        dimensions,
        OCRPageDimensions {
            dpi: None,
            height: None,
            width: None
        }
    );
}

#[test]
fn ocr_page_dimensions_drops_unknown_fields_without_extra_bag() {
    let dimensions: OCRPageDimensions =
        serde_json::from_str(r#"{"dpi": 300, "unexpected": "value"}"#).unwrap();
    assert_eq!(dimensions.dpi, Some(300));
    let dumped = serde_json::to_string(&dimensions).unwrap();
    assert!(!dumped.contains("unexpected"));
}

#[test]
fn ocr_page_image_keeps_unknown_fields_in_extra() {
    let image: OCRPageImage =
        serde_json::from_str(r#"{"image_base64": "YQ==", "provider_field": 7}"#).unwrap();
    assert_eq!(image.image_base64, Some("YQ==".to_owned()));
    assert_eq!(
        image.extra.get("provider_field"),
        Some(&Json::Number(serde_json::Number::from(7_i64)))
    );
    let dumped = serde_json::to_string(&image).unwrap();
    assert!(dumped.contains("provider_field"));
}

#[test]
fn ocr_page_keeps_unknown_fields_in_extra() {
    let page: OCRPage =
        serde_json::from_str(r#"{"index": 0, "markdown": "Title", "provider_field": "kept"}"#)
            .unwrap();
    assert_eq!(page.index, 0);
    assert_eq!(page.markdown, "Title");
    assert_eq!(
        page.extra.get("provider_field"),
        Some(&Json::Text("kept".to_owned()))
    );
}

#[test]
fn ocr_usage_info_keeps_unknown_fields_in_extra() {
    let usage: OCRUsageInfo =
        serde_json::from_str(r#"{"credits": 1.5, "provider_field": true}"#).unwrap();
    assert_eq!(usage.credits, Some(1.5_f64));
    assert_eq!(usage.extra.get("provider_field"), Some(&Json::Bool(true)));
}

#[test]
fn ocr_response_defaults_object_to_ocr_and_keeps_extra() {
    let response: OCRResponse = serde_json::from_str(
        r#"{"pages": [], "model": "mistral/mistral-ocr-latest", "provider_field": "kept"}"#,
    )
    .unwrap();
    assert_eq!(response.object, "ocr");
    assert_eq!(
        response.extra.get("provider_field"),
        Some(&Json::Text("kept".to_owned()))
    );
}

#[test]
fn ocr_response_renames_key_value_pairs_field() {
    let response: OCRResponse = serde_json::from_str(
        r#"{"pages": [], "model": "mistral/mistral-ocr-latest", "keyValuePairs": [{"name": "total"}]}"#,
    )
    .unwrap();
    let mut expected_pair = IndexMap::new();
    expected_pair.insert("name".to_owned(), Json::Text("total".to_owned()));
    assert_eq!(response.key_value_pairs, Some(vec![expected_pair]));
    let dumped = serde_json::to_string(&response).unwrap();
    assert!(dumped.contains("keyValuePairs"));
    assert!(!dumped.contains("key_value_pairs"));
}

#[test]
fn ocr_response_serializes_defaults_as_null() {
    let response: OCRResponse = serde_json::from_str(r#"{"pages": [], "model": "m"}"#).unwrap();
    let value = serde_json::to_value(&response).unwrap();
    let expected: Json = serde_json::from_str(
        r#"{
            "pages": [],
            "model": "m",
            "document_annotation": null,
            "usage_info": null,
            "content": null,
            "tables": null,
            "keyValuePairs": null,
            "object": "ocr"
        }"#,
    )
    .unwrap();
    assert_eq!(value, serde_json::to_value(&expected).unwrap());
}

#[test]
fn ocr_response_never_serializes_provider_native_response() {
    let mut response: OCRResponse =
        serde_json::from_str(r#"{"pages": [], "model": "mistral/mistral-ocr-latest"}"#).unwrap();
    response.provider_native_response = Some(Json::Text("native".to_owned()));
    let dumped = serde_json::to_string(&response).unwrap();
    assert!(!dumped.contains("provider_native_response"));
    assert!(!dumped.contains("_hidden_params"));
}
