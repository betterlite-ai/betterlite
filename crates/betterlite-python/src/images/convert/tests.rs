#![cfg(test)]

use super::super::edit::{edit, edit_file, form, parts, reply, status_message};
use super::super::generate::{generate, request};
use super::super::session::{
    ImageSession, call_setup, explicit_credentials, first_text, now_seconds,
};
use super::super::variation::{first_image, variation};
use super::fields::{boolean, headers, resolve_target, strings, target, text, timeout};
use super::options::{edit_options, generation_options};
use super::{
    ConvertError, DEFAULT_CONTENT_TYPE, DEFAULT_MASK_NAME, SettingsWire, image_file, images, parse,
};
use crate::completion::setup::outbound_client;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageFile;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_files::CallFile;
use betterlite_sdk::call_setup::RequestMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use std::collections::BTreeMap;
use std::time::Duration;

fn call(text: &str) -> (super::ImageCallWire, Vec<(String, CallFile)>) {
    let mut blobs = Vec::new();
    parse(&format!(r#"{{"arguments":{text}}}"#), &mut blobs).unwrap()
}

fn file(name: &str, bytes: u8) -> CallFile {
    CallFile {
        name: name.to_owned(),
        content_type: None,
        bytes: vec![bytes],
    }
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()
}

fn session() -> ImageSession {
    let environment = BTreeMap::new();
    let client = outbound_client(&environment).unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    ImageSession {
        environment,
        client,
        redactor,
    }
}

fn recorded_image(name: Option<&str>, bytes: u8) -> ImageFile {
    ImageFile {
        filename: name.map(str::to_owned),
        content: vec![bytes],
        content_type: None,
    }
}

fn edited(text: &str, drop: bool) -> Result<RequestMap, ConvertError> {
    let (wire, _files) = call(text);
    edit_options(&wire.arguments, "dall-e-2", drop)
}

#[test]
fn request_target_defaults_to_dall_e_2_on_openai() {
    let (wire, _files) = call(r#"{"prompt":"a cat"}"#);
    let (model, provider) = target(&wire.arguments).unwrap();
    assert_eq!(model, "dall-e-2");
    assert_eq!(provider, "openai");
}

#[test]
fn request_generation_model_prefix_names_the_provider() {
    let (model, provider) = resolve_target("openai/gpt-image-1", None);
    assert_eq!(model, "gpt-image-1");
    assert_eq!(provider, "openai");
}

#[test]
fn request_generation_options_keep_style_and_skip_control_names() {
    let (wire, _files) =
        call(r#"{"prompt":"a cat","style":"vivid","quality":"hd","timeout":30,"api_key":"sk-x"}"#);
    let options = generation_options(&wire.arguments, "dall-e-3", false).unwrap();
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
}

#[test]
fn error_generation_unsupported_param_fails_without_drop() {
    let (wire, _files) = call(r#"{"prompt":"a cat","style":"vivid"}"#);
    let error = generation_options(&wire.arguments, "dall-e-2", false).unwrap_err();
    assert!(error.to_string().contains("style"));
    let options = generation_options(&wire.arguments, "dall-e-2", true).unwrap();
    assert!(!options.contains_key("style"));
}

#[test]
fn request_edit_options_keep_the_declared_fields() {
    let options = edited(
        r#"{"background":"transparent","n":2,"size":"1024x1024","user":"me","mask":"m.png"}"#,
        false,
    )
    .unwrap();
    assert_eq!(
        options.get("background"),
        Some(&Json::Text(String::from("transparent")))
    );
    assert_eq!(options.get("n"), Some(&Json::Number(Number::from(2_u32))));
    assert_eq!(
        options.get("size"),
        Some(&Json::Text(String::from("1024x1024")))
    );
    assert_eq!(options.get("user"), Some(&Json::Text(String::from("me"))));
    assert!(!options.contains_key("mask"));
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
fn request_empty_file_name_takes_the_image_name() {
    let file = CallFile {
        name: String::new(),
        content_type: None,
        bytes: vec![0_u8],
    };
    let image = image_file(&file, "image.png");
    assert_eq!(image.filename.as_deref(), Some("image.png"));
}

#[test]
fn request_unnamed_mask_takes_the_mask_name() {
    let file = CallFile {
        name: String::from("file"),
        content_type: None,
        bytes: vec![0_u8],
    };
    let mask = images(&[(String::from("mask"), file)], "mask", DEFAULT_MASK_NAME)
        .pop()
        .unwrap();
    assert_eq!(mask.filename.as_deref(), Some("mask.png"));
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
fn request_text_preserves_empty_text_and_reports_wrong_types() {
    let (wire, _files) = call(r#"{"name":""}"#);
    assert_eq!(text(&wire.arguments, "name").unwrap().as_deref(), Some(""));
    assert_eq!(text(&wire.arguments, "missing").unwrap(), None);
    let (invalid, _) = call(r#"{"name":1}"#);
    assert_eq!(
        text(&invalid.arguments, "name").unwrap_err().to_string(),
        "name must be text"
    );
}

#[test]
fn request_boolean_preserves_false_and_checks_type() {
    let (wire, _files) = call(r#"{"enabled":false}"#);
    assert_eq!(boolean(&wire.arguments, "enabled").unwrap(), Some(false));
    let (invalid, _) = call(r#"{"enabled":"false"}"#);
    assert_eq!(
        boolean(&invalid.arguments, "enabled")
            .unwrap_err()
            .to_string(),
        "enabled must be a boolean"
    );
}

#[test]
fn request_strings_preserves_values_and_rejects_non_text_items() {
    let (wire, _files) = call(r#"{"names":["a","b"]}"#);
    assert_eq!(
        strings(&wire.arguments, "names").unwrap(),
        vec![String::from("a"), String::from("b")]
    );
    let (invalid, _) = call(r#"{"names":["a",1]}"#);
    assert_eq!(
        strings(&invalid.arguments, "names")
            .unwrap_err()
            .to_string(),
        "names must be a list of text values"
    );
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

#[test]
fn request_target_reads_the_model_and_custom_provider() {
    let (wire, _files) = call(r#"{"model":"gpt-image-1"}"#);
    let (model, provider) = target(&wire.arguments).unwrap();
    assert_eq!(model, "gpt-image-1");
    assert_eq!(provider, "openai");
    let (prefixed, _) = call(r#"{"model":"azure/gpt-image-1"}"#);
    let (stripped, found) = target(&prefixed.arguments).unwrap();
    assert_eq!(stripped, "gpt-image-1");
    assert_eq!(found, "azure");
}

#[test]
fn request_provider_matching_the_prefix_strips_only_its_own() {
    let (model, provider) = resolve_target("openai/gpt-image-1", Some("openai"));
    assert_eq!(model, "gpt-image-1");
    assert_eq!(provider, "openai");
    let (unprefixed, kept) = resolve_target("openai/gpt-image-1", Some("azure"));
    assert_eq!(unprefixed, "openai/gpt-image-1");
    assert_eq!(kept, "azure");
}

#[test]
fn request_timeout_reads_seconds_and_defaults_to_none() {
    let (wire, _files) = call(r#"{"timeout":30}"#);
    assert_eq!(
        timeout(&wire.arguments).unwrap(),
        Some(Duration::from_secs(30))
    );
    let (untimed, _) = call(r#"{"prompt":"a cat"}"#);
    assert_eq!(timeout(&untimed.arguments).unwrap(), None);
}

#[test]
fn request_generation_options_drop_the_additional_names() {
    let (wire, _files) =
        call(r#"{"style":"vivid","quality":"hd","additional_drop_params":["style"]}"#);
    let options = generation_options(&wire.arguments, "dall-e-3", false).unwrap();
    assert!(!options.contains_key("style"));
    assert_eq!(
        options.get("quality"),
        Some(&Json::Text(String::from("hd")))
    );
}

#[test]
fn request_generation_call_keeps_the_prompt() {
    let (wire, _files) = call(r#"{"prompt":"a cat"}"#);
    let built = request(&session(), &wire, "dall-e-2", "openai").unwrap();
    assert_eq!(built.prompt, "a cat");
    assert_eq!(built.model, "dall-e-2");
}

#[test]
fn request_edit_options_call_drop_params_removes_image_config() {
    let options = edited(
        r#"{"imageConfig":{"quality":"hd"},"drop_params":true}"#,
        false,
    )
    .unwrap();
    assert!(options.is_empty());
}

#[test]
fn request_edit_file_carries_the_content_and_name() {
    let image = recorded_image(Some("a.png"), 9);
    let converted = edit_file(&image);
    assert_eq!(converted.name.as_deref(), Some("a.png"));
    assert_eq!(converted.content, vec![9_u8]);
}

#[test]
fn request_edit_form_keeps_the_model_prompt_and_files() {
    let (wire, _files) = call(r#"{"prompt":"a cat","size":"1024x1024"}"#);
    let files = [
        (String::from("image"), file("first.png", 1)),
        (String::from("mask"), file("mask.png", 2)),
    ];
    let built = form(&wire, &files, "dall-e-2", "openai", &redactor()).unwrap();
    assert_eq!(
        built.data.get("model"),
        Some(&Json::Text(String::from("dall-e-2")))
    );
    assert_eq!(
        built.data.get("prompt"),
        Some(&Json::Text(String::from("a cat")))
    );
    assert_eq!(
        built.data.get("size"),
        Some(&Json::Text(String::from("1024x1024")))
    );
    assert_eq!(built.files.len(), 2);
    assert_eq!(built.files[0].0, "image");
    assert_eq!(built.files[0].1.filename.as_deref(), Some("first.png"));
    assert_eq!(built.files[0].1.content, vec![1_u8]);
    assert_eq!(built.files[1].0, "mask");
}

#[test]
fn request_edit_parts_carry_text_numbers_and_files() {
    let mut data = RequestMap::new();
    data.insert(String::from("model"), Json::Text(String::from("dall-e-2")));
    data.insert(String::from("n"), Json::Number(Number::from(2_u32)));
    data.insert(String::from("blank"), Json::Null);
    let images = vec![(String::from("image"), recorded_image(Some("a.png"), 3))];
    let found = parts(&data, &images);
    assert_eq!(found.len(), 4);
    assert_eq!(found[0].name, "model");
    assert_eq!(found[0].data, b"dall-e-2".to_vec());
    assert_eq!(found[1].name, "n");
    assert_eq!(found[1].data, b"2".to_vec());
    assert!(found[2].data.is_empty());
    assert_eq!(found[3].name, "image");
    assert_eq!(found[3].file_name.as_deref(), Some("a.png"));
    assert_eq!(found[3].data, vec![3_u8]);
}

#[test]
fn request_edit_reply_decodes_a_success_and_names_a_failure() {
    let ok = UpstreamResponse {
        status: 200,
        headers: http::HeaderMap::new(),
        body: br#"{"created":7,"data":[]}"#.to_vec(),
    };
    let decoded = reply(&ok, "dall-e-2", "openai", &redactor()).unwrap();
    assert_eq!(decoded.created, 7);
    let denied = UpstreamResponse {
        status: 400,
        headers: http::HeaderMap::new(),
        body: br#"{"error":{"message":"boom"}}"#.to_vec(),
    };
    let refusal = reply(&denied, "dall-e-2", "openai", &redactor()).unwrap_err();
    assert_eq!(refusal.message, "boom");
}

#[test]
fn request_status_message_reads_the_nested_error_or_the_body() {
    assert_eq!(status_message(br#"{"error":{"message":"boom"}}"#), "boom");
    assert_eq!(status_message(b"plain failure"), "plain failure");
}

#[tokio::test]
async fn error_generate_rejects_a_non_openai_provider() {
    let (wire, _files) = call(r#"{"model":"vertex_ai/foo"}"#);
    let image_session = session();
    let failure = generate(&image_session, wire).await.unwrap_err();
    assert!(
        failure
            .message
            .contains("image generation is not supported for vertex_ai")
    );
}

#[tokio::test]
async fn error_edit_rejects_a_non_openai_provider() {
    let (wire, files) = call(r#"{"model":"vertex_ai/foo"}"#);
    let image_session = session();
    let failure = edit(&image_session, wire, files).await.unwrap_err();
    assert!(
        failure
            .message
            .contains("image edit is not supported for vertex_ai")
    );
}

#[tokio::test]
async fn error_variation_rejects_a_non_openai_provider() {
    let (wire, files) = call(r#"{"model":"vertex_ai/foo"}"#);
    let image_session = session();
    let failure = variation(&image_session, wire, files, false)
        .await
        .unwrap_err();
    assert!(
        failure
            .message
            .contains("Invalid image variation provider: vertex_ai")
    );
}

#[test]
fn request_first_image_needs_an_image_and_keeps_its_name() {
    let missing = first_image(&[], "dall-e-2").unwrap_err();
    assert!(
        missing
            .message
            .contains("image is required for image variations")
    );
    let files = [(String::from("image"), file("a.jpg", 4))];
    let found = first_image(&files, "dall-e-2").unwrap();
    assert_eq!(found.filename.as_deref(), Some("a.jpg"));
    assert_eq!(found.content, vec![4_u8]);
}

#[tokio::test]
async fn error_variation_requires_an_api_key() {
    let text = r#"{"arguments":{"image":{"$file":{"name":"a.png","blob":0}}}}"#;
    let mut blobs = vec![vec![1_u8]];
    let (wire, files) = parse(text, &mut blobs).unwrap();
    let image_session = session();
    let failure = variation(&image_session, wire, files, false)
        .await
        .unwrap_err();
    assert!(
        failure
            .message
            .contains("API key is required for OpenAI image variations")
    );
}

#[test]
fn request_session_present_drops_an_empty_secret() {
    let (wire, _files) = call(r#"{"prompt":"a cat"}"#);
    let empty_api_key = SettingsWire {
        api_base: None,
        api_key: Some(SecretString::from("")),
        openai_key: None,
        drop_params: None,
    };
    let setup = call_setup(
        &session(),
        &wire.arguments,
        &empty_api_key,
        "dall-e-2",
        "openai",
    )
    .unwrap();
    assert!(setup.credentials.api_key.is_none());
    assert_eq!(setup.timeout, super::DEFAULT_TIMEOUT);
    let fallback = SettingsWire {
        api_base: None,
        api_key: Some(SecretString::from("")),
        openai_key: Some(SecretString::from("sk-x")),
        drop_params: None,
    };
    let kept = call_setup(&session(), &wire.arguments, &fallback, "dall-e-2", "openai").unwrap();
    assert_eq!(kept.credentials.api_key.unwrap().expose_secret(), "sk-x");
}

#[test]
fn request_image_session_build_reads_process_environment() {
    temp_env::with_var("B6043_IMAGE_TEST", Some("marker"), || {
        let image_session = ImageSession::build("{}").unwrap();
        assert_eq!(
            image_session
                .environment
                .get("B6043_IMAGE_TEST")
                .map(String::as_str),
            Some("marker")
        );
    });
}

#[test]
fn request_first_text_skips_an_empty_value() {
    let mut environment = BTreeMap::new();
    environment.insert(String::from("OPENAI_API_KEY"), String::new());
    environment.insert(
        String::from("OPENAI_API_BASE"),
        String::from("https://example.test"),
    );
    let found = first_text(&environment, &["OPENAI_API_KEY", "OPENAI_API_BASE"]);
    assert_eq!(found.as_deref(), Some("https://example.test"));
    assert_eq!(first_text(&BTreeMap::new(), &["OPENAI_API_KEY"]), None);
}

#[test]
fn request_now_seconds_is_the_current_epoch() {
    assert!(now_seconds() > 1_600_000_000);
}

#[test]
fn request_explicit_credentials_drop_empty_text() {
    let (wire, _files) = call(r#"{"api_base":"","api_key":"sk-x","organization":""}"#);
    let explicit = explicit_credentials(&wire.arguments).unwrap();
    assert_eq!(explicit.api_base, None);
    assert_eq!(explicit.organization, None);
    assert_eq!(explicit.api_key.unwrap().expose_secret(), "sk-x");
    let (given, _) = call(r#"{"api_base":"https://example.test","organization":"team"}"#);
    let resolved = explicit_credentials(&given.arguments).unwrap();
    assert_eq!(resolved.api_base.as_deref(), Some("https://example.test"));
    assert_eq!(resolved.organization.as_deref(), Some("team"));
    assert!(resolved.api_key.is_none());
}

#[rstest]
#[case::unauthorized(401, "AuthenticationError")]
#[case::server_error(500, "InternalServerError")]
fn error_edit_provider_class(#[case] status: u16, #[case] kind: &str) {
    let response = UpstreamResponse {
        status,
        headers: http::HeaderMap::new(),
        body: br#"{"error":{"message":"provider failure"}}"#.to_vec(),
    };
    let failure = reply(&response, "dall-e-2", "openai", &redactor()).unwrap_err();
    assert_eq!(failure.kind, kind);
    assert_eq!(failure.message, "provider failure");
}

#[rstest]
#[case::sync(false, "ValueError")]
#[case::asynchronous(true, "APIConnectionError")]
#[tokio::test]
async fn error_variation_missing_key_class(#[case] asynchronous: bool, #[case] kind: &str) {
    let (wire, files) = call(r#"{"image":"unused"}"#);
    let failure = variation(&session(), wire, files, asynchronous)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, kind);
    assert!(failure.message.contains("API key is required"));
}

#[test]
fn request_generation_call_drop_and_extra_body() {
    let (wire, _) =
        call(r#"{"style":"vivid","drop_params":true,"extra_body":{"seed":7},"unknown":"kept"}"#);
    let options = generation_options(&wire.arguments, "dall-e-2", false).unwrap();
    assert!(!options.contains_key("style"));
    let expected = RequestMap::from_iter([
        (String::from("seed"), Json::Number(Number::from(7_u32))),
        (String::from("unknown"), Json::Text(String::from("kept"))),
    ]);
    assert_eq!(options.get("extra_body"), Some(&Json::Object(expected)));
}

#[rstest]
#[case::provider(
    r#"{"model":"vertex_ai/foo"}"#,
    "vertex_ai",
    "Invalid image variation provider"
)]
#[case::file(
    r#"{"model":"dall-e-2","api_key":"key"}"#,
    "openai",
    "image is required"
)]
#[tokio::test]
async fn error_async_variation_local(
    #[case] input: &str,
    #[case] provider: &str,
    #[case] message: &str,
) {
    let (wire, files) = call(input);
    let failure = variation(&session(), wire, files, true).await.unwrap_err();
    assert_eq!(failure.kind, "APIConnectionError");
    assert_eq!(failure.provider.as_deref(), Some(provider));
    assert!(failure.message.contains(message));
}
