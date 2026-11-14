#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::Mutex;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::compatible_image_credentials::{
    AIML_DEFAULT_API_BASE, COMETAPI_DEFAULT_API_BASE, Host, Settings,
};
use betterlite_models::decisions::Json;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};

use super::credentials::{complete_url, validate_environment};
use super::error::{CompatibleImageReplyError, ImageCredentialError, ImageOptionError};
use super::options::{map_openai_params, request_body, supported_params};
use super::pricing::{ImageBilling, compatible_image_cost};
use super::reply::decode_compatible_image_reply;
use crate::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse, ImageResponse,
};

fn settings(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("an empty config loads")
        .sections
        .compatible_image_credentials
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn params(entries: &[(&str, &str)]) -> crate::api::image_contracts::records::ImageParams {
    entries
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_owned(),
                serde_json::from_str::<Json>(value).expect("a JSON fixture"),
            )
        })
        .collect()
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON fixture")
}

fn reply(status_code: u16, body: &str) -> ImageHttpResponse {
    ImageHttpResponse {
        status_code,
        headers: ImageHeaders::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn decode(
    host: Host,
    status_code: u16,
    body: &str,
) -> Result<ImageResponse, CompatibleImageReplyError> {
    decode_compatible_image_reply(host, &reply(status_code, body), ImageResponse::default())
}

fn images(response: &ImageResponse) -> &[ImageData] {
    response.data.as_deref().expect("a data list")
}

fn image(url: Option<&str>, b64_json: Option<&str>, revised_prompt: Option<&str>) -> ImageData {
    ImageData {
        b64_json: b64_json.map(str::to_owned),
        revised_prompt: revised_prompt.map(str::to_owned),
        url: url.map(str::to_owned),
    }
}

#[rstest]
#[case::aiml_openai_style("openai/gpt-image-2", &["n", "size", "quality", "response_format", "output_format", "background", "moderation", "output_compression"])]
#[case::aiml_uppercase_prefix_is_flux("OpenAI/gpt-image-2", &["n", "response_format", "size"])]
#[case::aiml_flux("flux-pro/v1.1", &["n", "response_format", "size"])]
#[case::aiml_prefix_alone("openai/", &["n", "size", "quality", "response_format", "output_format", "background", "moderation", "output_compression"])]
fn supported_params_split_on_the_case_sensitive_prefix(
    #[case] model: &str,
    #[case] expected: &[&str],
) {
    assert_eq!(supported_params(Host::Aiml, model), expected);
}

#[test]
fn cometapi_supports_five_params() {
    assert_eq!(
        supported_params(Host::CometApi, "any-model"),
        &["n", "quality", "response_format", "size", "style"]
    );
}

#[rstest]
#[case::n("n", "2", "num_images", "2")]
#[case::response_format("response_format", "b64_json", "output_format", "b64_json")]
#[case::size_string("size", "512", "image_size", "\"512\"")]
#[case::size_number("size", "512", "image_size", "512")]
fn flux_mapping_renames_without_parsing_plain_values(
    #[case] name: &str,
    #[case] value: &str,
    #[case] expected_name: &str,
    #[case] expected_value: &str,
) {
    let mut optional = params(&[]);
    map_openai_params(
        Host::Aiml,
        &params(&[(name, value)]),
        &mut optional,
        "flux-pro",
        false,
    )
    .expect("a supported option");
    assert_eq!(optional.len(), 1);
    assert_eq!(
        optional.get(expected_name),
        Some(&json(expected_value)),
        "{optional:?}"
    );
}

#[test]
fn flux_size_splits_on_x_into_two_integers() {
    let mut optional = params(&[]);
    map_openai_params(
        Host::Aiml,
        &params(&[("size", "\"1024x768\"")]),
        &mut optional,
        "flux-pro",
        false,
    )
    .expect("a supported size");
    assert_eq!(
        optional.get("image_size"),
        Some(&json(r#"{"width":1024,"height":768}"#))
    );
}

#[rstest]
#[case::three_parts("\"1x2x3\"")]
#[case::empty_parts("\"x\"")]
#[case::not_a_number("\"widexhigh\"")]
fn flux_size_that_does_not_split_into_two_integers_fails(#[case] size: &str) {
    let error = map_openai_params(
        Host::Aiml,
        &params(&[("size", size)]),
        &mut params(&[]),
        "flux-pro",
        false,
    )
    .expect_err("a malformed size");
    assert!(matches!(error, ImageOptionError::Size { .. }));
}

#[test]
fn preexisting_options_skip_the_support_check_and_stay_untouched() {
    let existing = params(&[("quality", "\"hd\""), ("num_images", "4")]);
    let mut optional = existing.clone();
    map_openai_params(
        Host::Aiml,
        &params(&[("quality", "\"low\""), ("n", "2")]),
        &mut optional,
        "flux-pro",
        false,
    )
    .expect("preexisting keys are skipped before support is checked");
    assert_eq!(optional, existing);
}

#[rstest]
#[case::raise(false)]
#[case::drop(true)]
fn unsupported_options_raise_unless_drop_params(#[case] drop_params: bool) {
    let outcome = map_openai_params(
        Host::Aiml,
        &params(&[("style", "\"vivid\"")]),
        &mut params(&[]),
        "flux-pro",
        drop_params,
    );
    match (drop_params, outcome) {
        (true, Ok(())) => {}
        (false, Err(ImageOptionError::Unsupported { name, .. })) => assert_eq!(name, "style"),
        (_, found) => panic!("unexpected outcome {found:?}"),
    }
}

#[test]
fn openai_style_aiml_passes_every_supported_option_through() {
    let mut optional = params(&[]);
    map_openai_params(
        Host::Aiml,
        &params(&[
            ("n", "2"),
            ("quality", "\"high\""),
            ("background", "\"opaque\""),
            ("output_compression", "70"),
        ]),
        &mut optional,
        "openai/gpt-image-2",
        false,
    )
    .expect("every openai-style option is supported");
    assert_eq!(
        optional,
        params(&[
            ("n", "2"),
            ("quality", "\"high\""),
            ("background", "\"opaque\""),
            ("output_compression", "70"),
        ])
    );
}

#[test]
fn cometapi_passes_supported_options_without_renaming() {
    let mut optional = params(&[]);
    map_openai_params(
        Host::CometApi,
        &params(&[("n", "2"), ("style", "\"vivid\""), ("size", "1024x768")]),
        &mut optional,
        "any-model",
        false,
    )
    .expect("every listed option is supported");
    assert_eq!(
        optional,
        params(&[("n", "2"), ("style", "\"vivid\""), ("size", "1024x768")])
    );
}

#[rstest]
#[case::openai_style(Host::Aiml, "openai/gpt-image-2")]
#[case::cometapi(Host::CometApi, "any-model")]
fn request_options_overlay_model_and_prompt(#[case] host: Host, #[case] model: &str) {
    let body = request_body(
        host,
        model,
        "a cat",
        &params(&[("size", "\"512\""), ("model", "\"other\"")]),
    )
    .expect("the dict merge lets an option win");
    assert_eq!(body.get("model"), Some(&json("\"other\"")));
    assert_eq!(body.get("prompt"), Some(&json("\"a cat\"")));
    assert_eq!(body.get("size"), Some(&json("\"512\"")));
}

#[rstest]
#[case::model_name("model")]
#[case::prompt_name("prompt")]
fn flux_request_body_rejects_a_duplicate_name(#[case] name: &str) {
    let error = request_body(
        Host::Aiml,
        "flux-pro",
        "a cat",
        &params(&[(name, "\"twice\"")]),
    )
    .expect_err("the TypedDict constructor refuses a repeated keyword");
    assert!(matches!(error, ImageOptionError::Duplicate { .. }));
}

#[test]
fn flux_request_body_keeps_the_supplied_order() {
    let body = request_body(
        Host::Aiml,
        "flux-pro",
        "a cat",
        &params(&[("image_size", "\"512\""), ("num_images", "2")]),
    )
    .expect("a flux body");
    assert_eq!(
        body.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["model", "prompt", "image_size", "num_images"]
    );
}

#[rstest]
#[case::aiml_default(Host::Aiml, &[], None, "https://api.aimlapi.com/v1/images/generations")]
#[case::aiml_environment(Host::Aiml, &[("AIML_API_BASE", "https://aiml.example")], None, "https://aiml.example/v1/images/generations")]
#[case::aiml_explicit_wins(Host::Aiml, &[("AIML_API_BASE", "https://aiml.example")], Some("https://explicit.example"), "https://explicit.example/v1/images/generations")]
#[case::aiml_trailing_slashes(Host::Aiml, &[], Some("https://explicit.example///"), "https://explicit.example/v1/images/generations")]
#[case::aiml_strips_v1(Host::Aiml, &[], Some("https://explicit.example/v1"), "https://explicit.example/v1/images/generations")]
#[case::aiml_strips_v1_after_slashes(Host::Aiml, &[], Some("https://explicit.example/v1/"), "https://explicit.example/v1/images/generations")]
#[case::aiml_empty_explicit(Host::Aiml, &[("AIML_API_BASE", "https://aiml.example")], Some(""), "https://aiml.example/v1/images/generations")]
#[case::aiml_empty_environment(Host::Aiml, &[("AIML_API_BASE", "")], None, "https://api.aimlapi.com/v1/images/generations")]
#[case::cometapi_default(Host::CometApi, &[], None, "https://api.cometapi.com/v1/images/generations")]
#[case::cometapi_base_url_alias(Host::CometApi, &[("COMETAPI_BASE_URL", "https://comet.example")], None, "https://comet.example/v1/images/generations")]
#[case::cometapi_api_base_alias(Host::CometApi, &[("COMETAPI_API_BASE", "https://alternate.example")], None, "https://alternate.example/v1/images/generations")]
#[case::cometapi_first_alias_wins(Host::CometApi, &[("COMETAPI_BASE_URL", "https://first.example"), ("COMETAPI_API_BASE", "https://second.example")], None, "https://first.example/v1/images/generations")]
#[case::cometapi_empty_first_alias(Host::CometApi, &[("COMETAPI_BASE_URL", ""), ("COMETAPI_API_BASE", "https://second.example")], None, "https://second.example/v1/images/generations")]
#[case::cometapi_keeps_v1(Host::CometApi, &[], Some("https://explicit.example/v1"), "https://explicit.example/v1/v1/images/generations")]
#[case::cometapi_trailing_slashes(Host::CometApi, &[], Some("https://explicit.example//"), "https://explicit.example/v1/images/generations")]
fn complete_url_resolution(
    #[case] host: Host,
    #[case] environment: &[(&str, &str)],
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        complete_url(host, &settings(environment), api_base),
        expected
    );
}

#[rstest]
#[case::explicit_aiml_key(Host::Aiml, &[], Some("explicit"), "Bearer explicit")]
#[case::primary_aiml_key(Host::Aiml, &[("AIML_API_KEY", "primary")], None, "Bearer primary")]
#[case::alternative_aiml_key(Host::Aiml, &[("AIMLAPI_KEY", "alternative")], None, "Bearer alternative")]
#[case::aiml_primary_wins(Host::Aiml, &[("AIML_API_KEY", "primary"), ("AIMLAPI_KEY", "alternative")], None, "Bearer primary")]
#[case::empty_aiml_primary_falls_through(Host::Aiml, &[("AIML_API_KEY", ""), ("AIMLAPI_KEY", "alternative")], None, "Bearer alternative")]
#[case::explicit_cometapi_key(Host::CometApi, &[], Some("explicit"), "Bearer explicit")]
#[case::primary_cometapi_key(Host::CometApi, &[("COMETAPI_KEY", "primary")], None, "Bearer primary")]
#[case::alternative_cometapi_key(Host::CometApi, &[("COMETAPI_API_KEY", "alternative")], None, "Bearer alternative")]
#[case::empty_cometapi_primary_falls_through(Host::CometApi, &[("COMETAPI_KEY", ""), ("COMETAPI_API_KEY", "alternative")], None, "Bearer alternative")]
#[case::empty_explicit_falls_through(Host::CometApi, &[("COMETAPI_KEY", "primary")], Some(""), "Bearer primary")]
fn key_aliases_and_explicit_precedence(
    #[case] host: Host,
    #[case] environment: &[(&str, &str)],
    #[case] api_key: Option<&str>,
    #[case] expected: &str,
) {
    let supplied = api_key.map(secret);
    let headers = validate_environment(
        host,
        &settings(environment),
        supplied.as_ref(),
        &ImageHeaders::new(),
    )
    .expect("a resolved key");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some(expected)
    );
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[rstest]
#[case::aiml(Host::Aiml)]
#[case::cometapi(Host::CometApi)]
fn missing_keys_are_credential_errors(#[case] host: Host) {
    let error = validate_environment(host, &settings(&[]), None, &ImageHeaders::new())
        .expect_err("no key anywhere");
    assert!(matches!(
        (host, error),
        (Host::Aiml, ImageCredentialError::MissingAimlKey)
            | (Host::CometApi, ImageCredentialError::MissingCometApiKey)
    ));
}

#[test]
fn environment_key_is_visible_through_the_loader() {
    let loaded = settings(&[("AIML_API_KEY", "primary"), ("COMETAPI_API_KEY", "comet")]);
    assert_eq!(
        loaded
            .aiml_api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("primary")
    );
    assert_eq!(
        loaded
            .cometapi_api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("comet")
    );
}

#[test]
fn headers_keep_caller_names_and_overwrite_auth_and_content_type() {
    let mut headers = ImageHeaders::new();
    headers.insert(String::from("accept"), String::from("text/plain"));
    headers.insert(String::from("Authorization"), String::from("Bearer caller"));
    headers.insert(String::from("Content-Type"), String::from("text/xml"));
    let validated = validate_environment(
        Host::Aiml,
        &settings(&[("AIML_API_KEY", "env")]),
        None,
        &headers,
    )
    .expect("a resolved key");
    assert_eq!(
        validated.get("accept").map(String::as_str),
        Some("text/plain")
    );
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer env")
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn aiml_data_url_wins_even_when_null() {
    let response = decode(
        Host::Aiml,
        200,
        r#"{"data":[{"url":null,"b64_json":"b64","revised_prompt":"paint"}]}"#,
    )
    .expect("a data reply");
    assert_eq!(images(&response), &[image(None, None, Some("paint"))]);
}

#[test]
fn aiml_data_base64_uses_the_truthy_fallback_and_keeps_revised_prompt() {
    let response = decode(
        Host::Aiml,
        200,
        r#"{"data":[{"b64_json":"","image_base64":"fallback","revised_prompt":"paint"},{"b64_json":"direct","image_base64":"other"},{"b64_json":"alone"},{"image_base64":"only"}]}"#,
    )
    .expect("a data reply");
    assert_eq!(
        images(&response),
        &[
            image(None, Some("fallback"), Some("paint")),
            image(None, Some("direct"), None),
            image(None, Some("alone"), None),
            image(None, Some("only"), None),
        ]
    );
}

#[test]
fn aiml_data_entries_without_image_fields_are_skipped() {
    let response = decode(
        Host::Aiml,
        200,
        r#"{"data":[{"width":1024,"height":768},{"url":"https://mock/a.png"}]}"#,
    )
    .expect("a data reply");
    assert_eq!(
        images(&response),
        &[image(Some("https://mock/a.png"), None, None)]
    );
}

#[test]
fn aiml_empty_data_suppresses_output_choices_and_images() {
    let response = decode(
        Host::Aiml,
        200,
        r#"{"data":[],"output":{"choices":[{"image_base64":"b64"}]},"images":[{"url":"https://mock/a.png"}]}"#,
    )
    .expect("an empty data reply");
    assert!(images(&response).is_empty());
}

#[test]
fn aiml_non_list_data_falls_through_to_choices() {
    let response = decode(
        Host::Aiml,
        200,
        r#"{"data":{},"output":{"choices":[{"image_base64":"b64"}]},"images":[{"url":"https://mock/a.png"}]}"#,
    )
    .expect("a choices reply");
    assert_eq!(images(&response), &[image(None, Some("b64"), None)]);
}

#[test]
fn aiml_choices_prefer_image_base64_and_drop_revised_prompt() {
    let response = decode(
        Host::Aiml,
        200,
        r#"{"output":{"choices":[{"image_base64":null,"url":"https://mock/a.png","revised_prompt":"paint"},{"url":"https://mock/b.png"}]}}"#,
    )
    .expect("a choices reply");
    assert_eq!(
        images(&response),
        &[
            image(None, None, None),
            image(Some("https://mock/b.png"), None, None),
        ]
    );
}

#[test]
fn aiml_images_prefer_url_and_drop_revised_prompt() {
    let response = decode(
        Host::Aiml,
        200,
        r#"{"images":[{"url":"https://mock/a.png","image_base64":"b64","revised_prompt":"paint"},{"image_base64":"b64"}]}"#,
    )
    .expect("an images reply");
    assert_eq!(
        images(&response),
        &[
            image(Some("https://mock/a.png"), None, None),
            image(None, Some("b64"), None),
        ]
    );
}

#[test]
fn existing_images_are_retained_and_new_ones_appended() {
    let existing = ImageResponse {
        created: 7,
        data: Some(vec![image(Some("https://mock/old.png"), None, Some("old"))]),
        usage: None,
    };
    let response = decode_compatible_image_reply(
        Host::Aiml,
        &reply(200, r#"{"data":[{"url":"https://mock/new.png"}]}"#),
        existing,
    )
    .expect("an appending reply");
    assert_eq!(
        images(&response),
        &[
            image(Some("https://mock/old.png"), None, Some("old")),
            image(Some("https://mock/new.png"), None, None),
        ]
    );
}

#[test]
fn an_empty_existing_list_becomes_the_appended_list() {
    let existing = ImageResponse {
        created: 7,
        data: Some(Vec::new()),
        usage: None,
    };
    let response = decode_compatible_image_reply(
        Host::Aiml,
        &reply(200, r#"{"images":[{"url":"https://mock/a.png"}]}"#),
        existing,
    )
    .expect("an appending reply");
    assert_eq!(
        images(&response),
        &[image(Some("https://mock/a.png"), None, None)]
    );
}

#[test]
fn cometapi_copies_both_nullable_fields_and_drops_revised_prompt() {
    let response = decode(
        Host::CometApi,
        200,
        r#"{"created":1,"data":[{"url":"https://mock/a.png","b64_json":"b64","revised_prompt":"paint"},{"url":null}]}"#,
    )
    .expect("a CometAPI reply");
    assert_eq!(
        images(&response),
        &[
            image(Some("https://mock/a.png"), Some("b64"), None),
            image(None, None, None),
        ]
    );
}

#[rstest]
#[case::missing_data("{}")]
#[case::other_layout_is_ignored(r#"{"images":[{"url":"https://mock/a.png"}]}"#)]
fn cometapi_leaves_the_response_alone(#[case] body: &str) {
    let response = decode(Host::CometApi, 200, body).expect("a reply without data");
    assert!(images(&response).is_empty());
}

#[test]
fn cometapi_data_must_be_a_list() {
    let error = decode(Host::CometApi, 200, r#"{"data":null}"#).expect_err("null data");
    assert!(matches!(
        error,
        CompatibleImageReplyError::List { field: "data" }
    ));
}

#[rstest]
#[case::not_an_object("[]")]
#[case::data_entry_not_an_object(r#"{"data":[5]}"#)]
#[case::data_url_not_text(r#"{"data":[{"url":5}]}"#)]
#[case::images_not_a_list(r#"{"images":"none"}"#)]
#[case::choices_not_a_list(r#"{"output":{"choices":5}}"#)]
fn malformed_shapes_name_the_field(#[case] body: &str) {
    let error = decode(Host::Aiml, 200, body).expect_err("a malformed shape");
    assert_eq!(error.status(), None, "{error}");
}

#[test]
fn json_failure_keeps_the_reply_status_and_headers() {
    let mut headers = ImageHeaders::new();
    headers.insert(String::from("retry-after"), String::from("2"));
    let response = ImageHttpResponse {
        status_code: 502,
        headers,
        body: b"<html>".to_vec(),
    };
    let error = decode_compatible_image_reply(Host::Aiml, &response, ImageResponse::default())
        .expect_err("a body that is not JSON");
    let CompatibleImageReplyError::Provider(provider) = &error else {
        panic!("the JSON failure is the provider error: {error}");
    };
    assert_eq!(provider.status_code, 502);
    assert_eq!(error.status(), Some(502));
    assert_eq!(
        provider.headers,
        Some(vec![(String::from("retry-after"), String::from("2"))])
    );
}

#[test]
fn a_reply_over_the_body_limit_is_refused() {
    let response = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: vec![b' '; super::reply::MAX_REPLY_BYTES.saturating_add(1)],
    };
    let error = decode_compatible_image_reply(Host::Aiml, &response, ImageResponse::default())
        .expect_err("an oversized reply");
    assert!(matches!(error, CompatibleImageReplyError::TooLarge { .. }));
}

#[test]
fn a_reply_at_the_body_limit_is_still_decoded() {
    let mut body = br#"{"data":[]}"#.to_vec();
    body.resize(super::reply::MAX_REPLY_BYTES, b' ');
    let response = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body,
    };
    let decoded =
        decode_compatible_image_reply(Host::CometApi, &response, ImageResponse::default())
            .expect("a reply of exactly the limit");
    assert!(images(&decoded).is_empty());
}

#[test]
fn created_and_usage_are_copied_from_the_supplied_response() {
    let existing: ImageResponse = serde_json::from_str(
        r#"{"created":1700000000,"data":null,"usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}}"#,
    )
    .expect("a response");
    let response =
        decode_compatible_image_reply(Host::CometApi, &reply(200, r#"{"data":[]}"#), existing)
            .expect("a CometAPI reply");
    assert_eq!(response.created, 1_700_000_000);
    let usage = response.usage.expect("usage");
    assert_eq!(usage.input_tokens, 2);
    assert_eq!(usage.output_tokens, 3);
    assert_eq!(usage.total_tokens, 5);
}

struct RecordingLookup {
    entries: BTreeMap<String, PriceRecord>,
    asked: Mutex<Vec<(String, Option<String>)>>,
}

impl RecordingLookup {
    fn new(entries: &[(&str, &str)]) -> Self {
        Self {
            entries: entries
                .iter()
                .map(|(key, record)| {
                    (
                        (*key).to_owned(),
                        serde_json::from_str(record).expect("a price record"),
                    )
                })
                .collect(),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn asked(&self) -> Vec<(String, Option<String>)> {
        self.asked.lock().expect("the recording lock").clone()
    }

    fn record(&self, model: &str, custom_llm_provider: Option<&str>) -> Option<PriceRecord> {
        self.asked
            .lock()
            .expect("the recording lock")
            .push((model.to_owned(), custom_llm_provider.map(str::to_owned)));
        let key = custom_llm_provider.map_or_else(
            || model.to_owned(),
            |provider| format!("{provider}/{model}"),
        );
        self.entries.get(&key).cloned()
    }
}

impl PriceLookup for RecordingLookup {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.record(model, custom_llm_provider)
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.record(model, custom_llm_provider)
            .map(|record| PricedModel {
                record,
                tiered_pricing: Vec::new(),
                off_peak_pricing: None,
                off_peak_cache_read_present: false,
            })
    }
}

fn billed(count: usize) -> ImageResponse {
    ImageResponse {
        created: 0,
        data: Some(vec![image(Some("https://mock/a.png"), None, None); count]),
        usage: None,
    }
}

#[rstest]
#[case::aiml(Host::Aiml, "aiml")]
#[case::cometapi(Host::CometApi, "cometapi")]
fn pricing_multiplies_the_host_rate_by_the_returned_count(
    #[case] host: Host,
    #[case] provider: &str,
) {
    let response = billed(2);
    let lookup = RecordingLookup::new(&[
        ("aiml/flux", r#"{"output_cost_per_image":0.25}"#),
        ("cometapi/flux", r#"{"output_cost_per_image":0.125}"#),
    ]);
    let cost = compatible_image_cost(
        host,
        "flux",
        ImageBilling::Response(&response),
        None,
        &lookup,
    )
    .expect("a priced image");
    let expected = if provider == "aiml" { "0.5" } else { "0.25" };
    assert_eq!(cost, Decimal::from_str(expected).expect("a decimal"));
    assert_eq!(
        lookup.asked(),
        vec![(String::from("flux"), Some(provider.to_owned()))]
    );
}

#[test]
fn a_missing_rate_counts_as_zero() {
    let lookup = RecordingLookup::new(&[("aiml/flux", r#"{"output_cost_per_token":0.1}"#)]);
    let cost = compatible_image_cost(
        Host::Aiml,
        "flux",
        ImageBilling::Response(&billed(3)),
        None,
        &lookup,
    )
    .expect("a zero rate");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn a_supplied_model_info_is_used_when_the_lookup_misses() {
    let record: PriceRecord =
        serde_json::from_str(r#"{"output_cost_per_image":0.5}"#).expect("a price record");
    let cost = compatible_image_cost(
        Host::CometApi,
        "unmapped",
        ImageBilling::Response(&billed(2)),
        Some(&record),
        &RecordingLookup::new(&[]),
    )
    .expect("the supplied table");
    assert_eq!(cost, Decimal::from_str("1").expect("a decimal"));
}

#[test]
fn an_unknown_model_without_model_info_is_an_error() {
    let error = compatible_image_cost(
        Host::Aiml,
        "unmapped",
        ImageBilling::Response(&billed(1)),
        None,
        &RecordingLookup::new(&[]),
    )
    .expect_err("no price table");
    assert!(matches!(error, ImageCostError::UnknownModel { .. }));
}

#[test]
fn a_non_image_billing_value_is_refused() {
    let lookup = RecordingLookup::new(&[("aiml/flux", r#"{"output_cost_per_image":0.25}"#)]);
    let error = compatible_image_cost(Host::Aiml, "flux", ImageBilling::Other, None, &lookup)
        .expect_err("only an ImageResponse is billed");
    assert!(matches!(error, ImageCostError::NotImageResponse));
}

#[test]
fn a_rate_of_the_wrong_type_names_the_field() {
    let lookup = RecordingLookup::new(&[("aiml/flux", r#"{"output_cost_per_image":"nope"}"#)]);
    let error = compatible_image_cost(
        Host::Aiml,
        "flux",
        ImageBilling::Response(&billed(1)),
        None,
        &lookup,
    )
    .expect_err("a string rate");
    assert!(matches!(error, ImageCostError::WrongFieldType { .. }));
}

fn check_seed(name: &str, bytes: &[u8], host: Host) -> Result<(), IoError> {
    let response = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: bytes.to_vec(),
    };
    let decoded = decode_compatible_image_reply(host, &response, ImageResponse::default());
    let must_decode = match host {
        Host::Aiml => name.starts_with("valid-aiml"),
        Host::CometApi => name.starts_with("valid-cometapi"),
    };
    assert!(!must_decode || decoded.is_ok(), "{name}: {decoded:?}");
    Ok(())
}

#[test]
fn replays_compatible_image_corpus() {
    let corpus =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/compatible_image_reply");
    let mut paths = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "no compatible image reply corpus seeds");
    paths
        .into_iter()
        .try_for_each(|path| {
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            [Host::Aiml, Host::CometApi]
                .iter()
                .try_for_each(|host| check_seed(&name, &bytes, *host))
        })
        .unwrap();
}
