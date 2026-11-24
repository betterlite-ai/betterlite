#![cfg(test)]

use super::fields::{generation_target, headers, resolve_target, timeout};
use super::options::{edit_options, generation_options};
use super::{ConvertError, DEFAULT_CONTENT_TYPE, image_file, parse};
use betterlite_models::decisions::Json;
use betterlite_sdk::call_files::CallFile;

fn call(text: &str) -> (super::ImageCallWire, Vec<(String, CallFile)>) {
    let mut blobs = Vec::new();
    parse(&format!(r#"{{"arguments":{text}}}"#), &mut blobs).unwrap()
}

fn edited(text: &str, drop: bool) -> Result<super::RequestMap, ConvertError> {
    let (wire, _files) = call(text);
    edit_options(&wire.arguments, "dall-e-2", drop)
}

#[test]
fn request_generation_target_defaults_to_dall_e_2_on_openai() {
    let (wire, _files) = call(r#"{"prompt":"a cat"}"#);
    let (model, provider) = generation_target(&wire.arguments).unwrap();
    assert_eq!(model, "dall-e-2");
    assert_eq!(provider, "openai");
}

#[test]
fn request_generation_model_prefix_names_the_provider() {
    let (model, provider) = resolve_target("openai/gpt-image-1", None).unwrap();
    assert_eq!(model, "gpt-image-1");
    assert_eq!(provider, "openai");
}

#[test]
fn request_generation_options_keep_style_and_skip_control_names() {
    let (wire, _files) = call(
        r#"{"prompt":"a cat","style":"vivid","quality":"hd","timeout":30,"api_key":"sk-x","presentation":"plain"}"#,
    );
    let options = generation_options(&wire.arguments);
    assert!(!options.contains_key("prompt"));
    assert!(!options.contains_key("timeout"));
    assert!(!options.contains_key("api_key"));
    assert_eq!(
        options.get("style"),
        Some(&Json::Text(String::from("vivid")))
    );
    assert_eq!(
        options.get("quality"),
        Some(&Json::Text(String::from("hd")))
    );
    assert_eq!(
        options.get("presentation"),
        Some(&Json::Text(String::from("plain")))
    );
}

#[test]
fn request_edit_options_keep_the_declared_fields() {
    let options = edited(
        r#"{"background":"transparent","n":2,"size":"1024x1024","user":"me"}"#,
        false,
    )
    .unwrap();
    assert_eq!(
        options.get("background"),
        Some(&Json::Text(String::from("transparent")))
    );
    assert_eq!(options.get("n"), Some(&Json::Number(2.into())));
    assert_eq!(
        options.get("size"),
        Some(&Json::Text(String::from("1024x1024")))
    );
    assert_eq!(options.get("user"), Some(&Json::Text(String::from("me"))));
}

#[test]
fn request_edit_options_drop_the_additional_names_first() {
    let options = edited(
        r#"{"size":"1024x1024","additional_drop_params":["size"]}"#,
        false,
    )
    .unwrap();
    assert!(!options.contains_key("size"));
}

#[test]
fn request_edit_options_unsupported_image_config_fails_without_drop() {
    let error = edited(r#"{"imageConfig":{"quality":"hd"}}"#, false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "The following parameters are not supported for model dall-e-2: imageConfig"
    );
}

#[test]
fn request_edit_options_drop_params_removes_image_config() {
    let options = edited(r#"{"imageConfig":{"quality":"hd"}}"#, true).unwrap();
    assert!(options.is_empty());
}

#[test]
fn request_edit_options_silently_skip_unknown_kwargs() {
    let options = edited(r#"{"output_compression":80,"seed":1}"#, false).unwrap();
    assert!(options.is_empty());
}

#[test]
fn request_files_keep_their_order_and_bytes() {
    let text = r#"{"image":[{"$file":{"name":"first.png","blob":0}},{"$file":{"name":"second.jpg","blob":1}}]}"#;
    let mut blobs = vec![vec![1_u8], vec![2_u8]];
    let (wire, files) = parse(&format!(r#"{{"arguments":{text}}}"#), &mut blobs).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].0, "image");
    assert_eq!(files[0].1.name, "first.png");
    assert_eq!(files[0].1.bytes, vec![1_u8]);
    assert_eq!(files[1].1.name, "second.jpg");
    assert_eq!(files[1].1.bytes, vec![2_u8]);
    assert_eq!(wire.arguments.len(), 1);
}

#[test]
fn request_unnamed_file_takes_the_image_name() {
    let file = CallFile {
        name: String::from("file"),
        content_type: None,
        bytes: vec![0_u8],
    };
    let image = image_file(&file, "image.png");
    assert_eq!(image.filename.as_deref(), Some("image.png"));
    assert_eq!(image.content_type.as_deref(), Some("image/png"));
}

#[test]
fn request_content_type_comes_from_the_name() {
    let jpeg = CallFile {
        name: String::from("photo.jpg"),
        content_type: None,
        bytes: Vec::new(),
    };
    assert_eq!(
        image_file(&jpeg, "image.png").content_type.as_deref(),
        Some("image/jpeg")
    );
    let unknown = CallFile {
        name: String::from("photo.unknownext"),
        content_type: None,
        bytes: Vec::new(),
    };
    assert_eq!(
        image_file(&unknown, "image.png").content_type.as_deref(),
        Some(DEFAULT_CONTENT_TYPE)
    );
}

#[test]
fn request_headers_merge_with_extra_headers_winning() {
    let (wire, _files) = call(r#"{"headers":{"x-a":"1"},"extra_headers":{"x-a":"2","x-b":"3"}}"#);
    let merged = headers(&wire.arguments).unwrap();
    assert_eq!(merged.get("x-a").map(String::as_str), Some("2"));
    assert_eq!(merged.get("x-b").map(String::as_str), Some("3"));
}

#[test]
fn error_wrong_field_type_names_the_field() {
    let (wire, _files) = call(r#"{"timeout":"soon"}"#);
    let error = timeout(&wire.arguments).unwrap_err();
    assert_eq!(error.to_string(), "timeout must be a number");
}

#[test]
fn error_unsupported_params_names_every_unsupported_field() {
    let error = edited(r#"{"imageConfig":{"a":1},"background":"auto"}"#, false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "The following parameters are not supported for model dall-e-2: imageConfig"
    );
}

#[test]
fn error_missing_blob_index_is_reported() {
    let mut blobs: Vec<Vec<u8>> = Vec::new();
    let error = parse(
        r#"{"arguments":{"image":{"$file":{"name":"a.png"}}}}"#,
        &mut blobs,
    )
    .unwrap_err();
    assert!(error.to_string().contains("no blob index"));
}
