#![cfg(test)]

use std::fmt::Debug;
use std::sync::atomic::Ordering;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageClassEntry, ImageClassKind, ImageConfig, ImageConfigValue, ImageFile, ImageHeaders,
    ImageHttpResponse, ImageParams, ImageResponse,
};
use betterlite_openai::error::ProviderError;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use super::edit::{BaseImageEditConfig, ImageEditEnvironment, ImageEditRequest};
use super::error::{ImageContractError, NOT_IMPLEMENTED_REQUEST, NOT_IMPLEMENTED_RESPONSE};
use super::generation::{BaseImageGenerationConfig, ImageGenerationReply, ImageGenerationRequest};
use super::shared::{ImageEnvironment, ImageUrlRequest};
use super::variation::{BaseImageVariationConfig, ImageVariationChat, ImageVariationReply};

use fixtures::Fixture;

mod fixtures;

fn params_from(entries: [(&str, Json); 1]) -> ImageParams {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn headers_from(entries: [(&str, &str); 1]) -> ImageHeaders {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

fn url_request<'request>(
    api_base: Option<&'request str>,
    optional_params: &'request ImageParams,
    litellm_params: &'request ImageParams,
) -> ImageUrlRequest<'request> {
    ImageUrlRequest {
        api_base,
        api_key: None,
        model: "dall-e-2",
        optional_params,
        litellm_params,
        stream: None,
    }
}

fn environment<'request>(
    headers: &'request ImageHeaders,
    optional_params: &'request ImageParams,
    litellm_params: &'request ImageParams,
) -> ImageEnvironment<'request> {
    ImageEnvironment {
        headers,
        model: "dall-e-2",
        messages: &[],
        optional_params,
        litellm_params,
        api_key: None,
        api_base: None,
    }
}

fn seed_image() -> ImageFile {
    ImageFile {
        filename: Some(String::from("seed.png")),
        content: vec![b's'],
        content_type: Some(String::from("image/png")),
    }
}

fn http_response() -> ImageHttpResponse {
    ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: Vec::new(),
    }
}

fn class_entry(name: &str, kind: ImageClassKind, value: Json, inherited: bool) -> ImageClassEntry {
    ImageClassEntry {
        name: name.to_owned(),
        kind,
        value,
        inherited,
    }
}

fn assert_edit_urls(fixture: &Fixture, api_base: Option<&str>, expected: &str) {
    let params = ImageParams::new();
    let edit = BaseImageEditConfig::get_complete_url(fixture, "dall-e-2", api_base, &params);
    match api_base {
        None => assert_eq!(
            edit.expect_err("required").to_string(),
            "api_base is required"
        ),
        Some(_) => assert_eq!(edit.expect("base"), expected),
    }
}

fn assert_edit_transport(fixture: &Fixture, headers: &ImageHeaders) {
    let edit_environment = ImageEditEnvironment {
        headers,
        model: "dall-e-2",
        api_key: None,
        litellm_params: None,
        api_base: None,
    };
    assert!(BaseImageEditConfig::validate_environment(fixture, edit_environment).is_empty());
    assert!(BaseImageEditConfig::use_multipart_form_data(fixture));
    let data = params_from([("prompt", Json::Text(String::from("p")))]);
    let finalized = BaseImageEditConfig::finalize_image_edit_request_data(
        fixture,
        data.clone(),
        "https://mock/edit",
    );
    assert_eq!(finalized, data);
}

#[rstest]
#[case::none(None, "")]
#[case::empty_base(Some(""), "")]
#[case::explicit_base(Some("https://mock/v1"), "https://mock/v1")]
fn image_transport_defaults(#[case] api_base: Option<&str>, #[case] expected: &str) {
    let fixture = Fixture::default();
    let params = ImageParams::new();
    let headers = headers_from([("Authorization", "secret")]);
    let request = url_request(api_base, &params, &params);
    assert_eq!(
        BaseImageGenerationConfig::get_complete_url(&fixture, request),
        expected
    );
    assert_eq!(
        BaseImageVariationConfig::get_complete_url(&fixture, request),
        expected
    );
    let generation = environment(&headers, &params, &params);
    assert!(BaseImageGenerationConfig::validate_environment(&fixture, generation).is_empty());
    assert!(BaseImageVariationConfig::validate_environment(&fixture, generation).is_empty());
    assert_edit_urls(&fixture, api_base, expected);
    assert_edit_transport(&fixture, &headers);
    assert!(!BaseImageGenerationConfig::use_multipart_form_data(
        &fixture
    ));
}

fn assert_generation_error(fixture: &Fixture, headers: &ImageHeaders) -> ProviderError {
    let generation = BaseImageGenerationConfig::get_error_class(fixture, "slow", 429, headers);
    assert_eq!(generation.status_code, 429);
    assert_eq!(generation.message, "slow");
    assert_eq!(
        generation.headers,
        Some(vec![(String::from("retry-after"), String::from("2"))])
    );
    assert!(!generation.status_code_is_synthesized);
    assert_eq!(generation.request.url, "https://docs.litellm.ai/docs");
    assert_eq!(generation.response.status_code, 429);
    assert_eq!(generation.body, None);
    generation
}

fn assert_edit_error(fixture: &Fixture, headers: &ImageHeaders, expected: &ProviderError) {
    let edit = BaseImageEditConfig::get_error_class(fixture, "slow", 429, headers);
    assert_eq!(edit.status_code, 429);
    assert_eq!(edit.message, "slow");
    assert_eq!(edit.headers, expected.headers);
}

fn assert_raises<T>(result: Result<T, ImageContractError>, message: &str)
where
    T: Debug,
{
    assert_eq!(result.expect_err("unimplemented").to_string(), message);
}

fn assert_generation_messages(fixture: &Fixture) {
    let params = ImageParams::new();
    let headers = ImageHeaders::new();
    let request = ImageGenerationRequest {
        model: "dall-e-2",
        prompt: "a cat",
        optional_params: &params,
        litellm_params: &params,
        headers: &headers,
    };
    assert_raises(
        BaseImageGenerationConfig::transform_image_generation_request(fixture, request),
        NOT_IMPLEMENTED_REQUEST,
    );
    let mut response = ImageResponse::default();
    let raw_response = http_response();
    let reply = ImageGenerationReply {
        model: "dall-e-2",
        raw_response: &raw_response,
        model_response: &mut response,
        request_data: &params,
        optional_params: &params,
        litellm_params: &params,
        api_key: None,
        json_mode: None,
    };
    assert_raises(
        BaseImageGenerationConfig::transform_image_generation_response(fixture, reply),
        NOT_IMPLEMENTED_RESPONSE,
    );
}

fn assert_variation_messages(fixture: &Fixture) {
    let params = ImageParams::new();
    let headers = ImageHeaders::new();
    let chat = ImageVariationChat {
        model: "dall-e-2",
        messages: &[],
        optional_params: &params,
        litellm_params: &params,
        headers: &headers,
    };
    assert_raises(
        BaseImageVariationConfig::transform_request(fixture, chat),
        NOT_IMPLEMENTED_REQUEST,
    );
    let image = seed_image();
    let api_key = SecretString::from("key");
    let mut response = ImageResponse::default();
    let raw_response = http_response();
    let reply = ImageVariationReply {
        model: None,
        raw_response: &raw_response,
        model_response: &mut response,
        request_data: &params,
        image: &image,
        optional_params: &params,
        litellm_params: &params,
        api_key: Some(&api_key),
    };
    assert_raises(
        BaseImageVariationConfig::transform_response(fixture, reply),
        NOT_IMPLEMENTED_RESPONSE,
    );
}

fn assert_unimplemented_messages(fixture: &Fixture) {
    assert_generation_messages(fixture);
    assert_variation_messages(fixture);
}

#[test]
fn image_base_errors() {
    let fixture = Fixture::default();
    let headers = headers_from([("retry-after", "2")]);
    let generation = assert_generation_error(&fixture, &headers);
    assert_edit_error(&fixture, &headers, &generation);
    assert_unimplemented_messages(&fixture);
}

fn edit_request(
    options: ImageParams,
    headers: ImageHeaders,
    litellm_params: &ImageParams,
) -> ImageEditRequest<'_> {
    ImageEditRequest {
        model: "gpt-image-1",
        prompt: None,
        image: None,
        optional_params: options,
        litellm_params,
        headers,
    }
}

#[tokio::test]
async fn edit_async_transform_copy() {
    let fixture = Fixture::default();
    let litellm_params = ImageParams::new();
    let options = params_from([("n", Json::Number(Number::from(2_i32)))]);
    let request_headers = headers_from([("x", "a")]);
    let copies = edit_request(options.clone(), request_headers.clone(), &litellm_params);
    let (data, files) = fixture
        .async_transform_image_edit_request(copies)
        .await
        .expect("copy");
    let seen_items = fixture.seen();
    assert_eq!(seen_items.len(), 1);
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
    let first_seen = &seen_items[0];
    assert_eq!(first_seen.model, "gpt-image-1");
    assert_eq!(first_seen.prompt, None);
    assert_eq!(first_seen.image, None);
    assert_eq!(first_seen.options, options);
    assert_eq!(first_seen.headers, request_headers);
    assert_eq!(first_seen.litellm_params, litellm_params);
    assert_eq!(data.get("n"), Some(&Json::Number(Number::from(2_i32))));
    assert_eq!(data.get("touched"), Some(&Json::Bool(true)));
    assert_eq!(data.get("header_touched"), Some(&Json::Bool(true)));
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, "image");
    let raising = Fixture::raising(418);
    let raising_copies = edit_request(options.clone(), request_headers.clone(), &litellm_params);
    let error = raising
        .async_transform_image_edit_request(raising_copies)
        .await
        .expect_err("raised");
    assert_eq!(error.to_string(), "the fixture provider failed");
    assert_eq!(raising.calls.load(Ordering::Relaxed), 1);
    assert_eq!(raising.seen()[0].options, options);
}

fn class_entries() -> Vec<ImageClassEntry> {
    let values = [
        ("n", Json::Number(Number::from(0_i32)), false),
        ("enabled", Json::Bool(false), false),
        ("label", Json::Text(String::new()), false),
        ("none", Json::Null, false),
        ("__private", Json::Number(Number::from(1_i32)), false),
        ("_abc_x", Json::Number(Number::from(2_i32)), false),
        ("size", Json::Number(Number::from(3_i32)), true),
    ];
    let kinds = [
        ("method", ImageClassKind::Function),
        ("builtin", ImageClassKind::BuiltinFunction),
        ("class_method", ImageClassKind::ClassMethod),
        ("static_method", ImageClassKind::StaticMethod),
        ("prop", ImageClassKind::Property),
        ("callable", ImageClassKind::CallableObject),
    ];
    let mut entries: Vec<ImageClassEntry> = values
        .into_iter()
        .map(|(name, value, inherited)| class_entry(name, ImageClassKind::Value, value, inherited))
        .collect();
    entries.extend(
        kinds
            .into_iter()
            .map(|(name, kind)| class_entry(name, kind, Json::Null, false)),
    );
    entries
}

fn assert_class_config(config: &ImageConfig) {
    assert_eq!(
        config.keys().map(String::as_str).collect::<Vec<_>>(),
        ["n", "enabled", "label", "prop", "callable"]
    );
    assert_eq!(
        config["n"],
        ImageConfigValue::Value {
            value: Json::Number(Number::from(0_i32))
        }
    );
    assert_eq!(
        config["enabled"],
        ImageConfigValue::Value {
            value: Json::Bool(false)
        }
    );
    assert_eq!(
        config["label"],
        ImageConfigValue::Value {
            value: Json::Text(String::new())
        }
    );
    assert_eq!(config["prop"], ImageConfigValue::Property);
    assert_eq!(config["callable"], ImageConfigValue::CallableObject);
}

#[test]
fn edit_class_configuration() {
    let entries = class_entries();
    let config = <Fixture as BaseImageEditConfig>::get_config(&entries);
    assert_class_config(&config);
}
