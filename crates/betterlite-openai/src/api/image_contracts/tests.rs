#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use super::records::{
    ImageClassEntry, ImageClassKind, ImageConfig, ImageConfigValue, ImageData, ImageFile,
    ImageFiles, ImageParams, ImageResponse,
};

#[test]
fn image_response_defaults_to_an_empty_reply() {
    let response = ImageResponse::default();
    assert_eq!(response.created, 0);
    assert_eq!(response.data, None);
    assert_eq!(response.usage, None);
}

#[test]
fn image_response_keeps_the_provider_images() {
    let response: ImageResponse = serde_json::from_str(
        r#"{"created":7,"data":[{"url":"https://mock/a.png","b64_json":"YQ==","revised_prompt":"paint"}],"usage":{"input_tokens":2,"input_tokens_details":{"image_tokens":1,"text_tokens":1},"output_tokens":3,"total_tokens":5}}"#,
    )
    .expect("response");
    assert_eq!(response.created, 7);
    assert_eq!(
        response.data,
        Some(vec![ImageData {
            b64_json: Some(String::from("YQ==")),
            revised_prompt: Some(String::from("paint")),
            url: Some(String::from("https://mock/a.png")),
        }])
    );
    let usage = response.usage.expect("usage");
    assert_eq!(usage.input_tokens, 2);
    assert_eq!(usage.input_tokens_details.image_tokens, 1);
    assert_eq!(usage.input_tokens_details.text_tokens, 1);
    assert_eq!(usage.output_tokens, 3);
    assert_eq!(usage.total_tokens, 5);
}

#[test]
fn image_data_writes_only_the_fields_it_holds() {
    let data = ImageData {
        b64_json: None,
        revised_prompt: None,
        url: Some(String::from("https://mock/a.png")),
    };
    assert_eq!(
        serde_json::to_string(&data).expect("data"),
        r#"{"url":"https://mock/a.png"}"#
    );
}

#[rstest]
#[case::value("value", ImageClassKind::Value)]
#[case::function("function", ImageClassKind::Function)]
#[case::builtin("builtin_function", ImageClassKind::BuiltinFunction)]
#[case::class_method("class_method", ImageClassKind::ClassMethod)]
#[case::static_method("static_method", ImageClassKind::StaticMethod)]
#[case::property("property", ImageClassKind::Property)]
#[case::callable("callable_object", ImageClassKind::CallableObject)]
fn image_class_entry_reads_each_kind(#[case] tag: &str, #[case] kind: ImageClassKind) {
    let entry: ImageClassEntry = serde_json::from_str(&format!(
        r#"{{"name":"key","kind":"{tag}","value":null,"inherited":false}}"#
    ))
    .expect("entry");
    assert_eq!(entry.kind, kind);
    assert_eq!(entry.value, Json::Null);
    assert!(!entry.inherited);
}

#[test]
fn image_config_values_tag_descriptors() {
    let mut config = ImageConfig::new();
    config.insert(
        String::from("n"),
        ImageConfigValue::Value {
            value: Json::Number(Number::from(0_i32)),
        },
    );
    config.insert(String::from("prop"), ImageConfigValue::Property);
    config.insert(String::from("callable"), ImageConfigValue::CallableObject);
    assert_eq!(
        serde_json::to_string(&config).expect("config"),
        r#"{"n":{"kind":"value","value":0},"prop":{"kind":"property"},"callable":{"kind":"callable_object"}}"#
    );
}

#[test]
fn image_file_parts_keep_their_name_and_content() {
    let files: ImageFiles = vec![(
        String::from("image"),
        ImageFile {
            filename: Some(String::from("a.png")),
            content: vec![b'a'],
            content_type: Some(String::from("image/png")),
        },
    )];
    assert_eq!(files.len(), 1);
    let (name, file) = &files[0];
    assert_eq!(name, "image");
    assert_eq!(file.filename.as_deref(), Some("a.png"));
    assert_eq!(file.content, vec![b'a']);
    assert_eq!(file.content_type.as_deref(), Some("image/png"));
    let params = ImageParams::new();
    assert!(params.is_empty());
}
