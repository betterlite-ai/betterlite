#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_config::types::RoutingStrategy;
use betterlite_models::decisions::Json;
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageFile, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse, ImageUsage,
};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use super::cost::cost_calculator;
use super::edit::RecraftImageEditConfig;
use super::error::RecraftError;
use super::generation::{RecraftImageGenerationConfig, get_recraft_image_generation_config};
use super::reply::{RecraftReplyError, decode_recraft_images};
use super::settings::RecraftImageSettings;
use crate::cost::image_dispatch::policy::ImageResponseView;

const JPEG: &[u8] = b"\xff\xd8\xff\xe0the rest of the jpeg";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfiller";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn generation(entries: &[(&str, &str)]) -> RecraftImageGenerationConfig {
    get_recraft_image_generation_config("recraft/recraftv3", &lookup(entries))
}

fn edit(entries: &[(&str, &str)]) -> RecraftImageEditConfig {
    let (api_key, api_base) = RoutingStrategy::recraft_image_settings(&lookup(entries));
    RecraftImageEditConfig {
        settings: RecraftImageSettings { api_key, api_base },
    }
}

fn params(entries: &[(&str, Json)]) -> ImageParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn integer(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture is JSON")
}

fn upstream(status: u16, body: &str) -> ImageHttpResponse {
    ImageHttpResponse {
        status_code: status,
        headers: ImageHeaders::from([("x-request-id".to_owned(), "req".to_owned())]),
        body: body.as_bytes().to_vec(),
    }
}

fn billing(image_count: i64) -> ImageBilling {
    ImageBilling {
        image_count,
        size: None,
        quality: None,
        usage: None,
    }
}

fn rate(value: f64) -> PriceRecord {
    PriceRecord::from([(
        "output_cost_per_image".to_owned(),
        Scalar::Number(Number::from_f64(value).expect("the test rate is finite")),
    )])
}

struct NoPrices;

impl PriceLookup for NoPrices {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

fn serialized<T>(value: &T) -> Json
where
    T: serde::Serialize,
{
    json(&serde_json::to_string(value).expect("the value serializes"))
}

#[test]
fn the_supported_openai_params_are_the_source_lists() {
    assert_eq!(
        RecraftImageGenerationConfig::SUPPORTED_OPENAI_PARAMS,
        ["n", "response_format", "size", "style"]
    );
    assert_eq!(
        RecraftImageEditConfig::SUPPORTED_OPENAI_PARAMS,
        ["n", "response_format", "style"]
    );
}

#[test]
fn generation_fills_only_the_missing_supported_options() {
    let non_default = params(&[
        ("n", integer(2)),
        ("size", text("1024x1024")),
        ("style", text("realistic_image")),
        ("response_format", text("b64_json")),
    ]);

    let mapped = RecraftImageGenerationConfig::map_openai_params(
        &non_default,
        params(&[("n", integer(9))]),
        "recraft/recraftv3",
        false,
    )
    .expect("every name is supported");

    assert_eq!(
        mapped,
        params(&[
            ("n", integer(9)),
            ("size", text("1024x1024")),
            ("style", text("realistic_image")),
            ("response_format", text("b64_json")),
        ])
    );
}

#[test]
fn generation_rejects_an_unsupported_option_unless_dropped() {
    let non_default = params(&[("quality", text("hd"))]);

    let error = RecraftImageGenerationConfig::map_openai_params(
        &non_default,
        ImageParams::new(),
        "recraft/recraftv3",
        false,
    )
    .expect_err("quality is unsupported");

    assert_eq!(
        error,
        RecraftError::UnsupportedParameter {
            parameter: "quality".to_owned(),
            model: "recraft/recraftv3".to_owned(),
        }
    );
    let dropped = RecraftImageGenerationConfig::map_openai_params(
        &non_default,
        ImageParams::new(),
        "recraft/recraftv3",
        true,
    )
    .expect("drop_params drops it");
    assert!(dropped.is_empty());
}

#[test]
fn generation_writes_prompt_model_and_every_provider_option() {
    let options = params(&[
        ("text_layout", json("[]")),
        ("n", integer(2)),
        ("style_id", text("style-1")),
        ("style", text("realistic_image")),
        ("substyle", text("digital_illustration")),
        ("response_format", text("url")),
        ("size", text("1024x1024")),
        ("negative_prompt", text("blurry")),
        ("controls", json(r#"{"colors":[]}"#)),
    ]);

    let request = RecraftImageGenerationConfig::transform_image_generation_request(
        "recraft/recraftv3",
        "a cat",
        &options,
    )
    .expect("the request is built");

    assert_eq!(
        serialized(&request),
        json(
            r#"{"prompt":"a cat","model":"recraft/recraftv3","text_layout":[],"n":2,"style_id":"style-1","style":"realistic_image","substyle":"digital_illustration","response_format":"url","size":"1024x1024","negative_prompt":"blurry","controls":{"colors":[]}}"#
        )
    );
}

#[rstest]
#[case::prompt("prompt")]
#[case::model("model")]
fn generation_rejects_a_duplicate_argument(#[case] argument: &'static str) {
    let error = RecraftImageGenerationConfig::transform_image_generation_request(
        "recraft/recraftv3",
        "a cat",
        &params(&[(argument, text("supplied"))]),
    )
    .expect_err("the TypedDict call raises");

    assert_eq!(error, RecraftError::DuplicateArgument { argument });
}

#[rstest]
#[case::default_base(&[], None, "https://external.api.recraft.ai")]
#[case::environment_base(
    &[("RECRAFT_API_BASE", "https://env.test/v1")],
    None,
    "https://env.test/v1"
)]
#[case::explicit_base_wins(
    &[("RECRAFT_API_BASE", "https://env.test/v1")],
    Some("https://caller.test/base"),
    "https://caller.test/base"
)]
#[case::empty_base_falls_through(
    &[("RECRAFT_API_BASE", "https://env.test/v1")],
    Some(""),
    "https://env.test/v1"
)]
#[case::trailing_slashes_are_trimmed(&[], Some("https://caller.test///"), "https://caller.test")]
fn the_generation_url_appends_its_endpoint_after_trimming_slashes(
    #[case] environment: &[(&str, &str)],
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        generation(environment).get_complete_url(api_base),
        format!("{expected}/v1/images/generations")
    );
}

#[test]
fn the_edit_url_appends_its_own_endpoint() {
    assert_eq!(
        edit(&[]).get_complete_url(None),
        "https://external.api.recraft.ai/v1/images/imageToImage"
    );
    assert_eq!(
        edit(&[("RECRAFT_API_BASE", "https://env.test/")]).get_complete_url(None),
        "https://env.test/v1/images/imageToImage"
    );
    assert_eq!(
        edit(&[]).get_complete_url(Some("https://caller.test/base/")),
        "https://caller.test/base/v1/images/imageToImage"
    );
    assert_eq!(
        edit(&[("RECRAFT_API_BASE", "https://env.test/v1")]).get_complete_url(Some("")),
        "https://env.test/v1/v1/images/imageToImage"
    );
}

#[test]
fn validate_environment_overwrites_the_authorization_header() {
    let mut headers = ImageHeaders::new();
    headers.insert("Authorization".to_owned(), "Bearer stale".to_owned());
    headers.insert("x-extra".to_owned(), "kept".to_owned());

    let validated = generation(&[("RECRAFT_API_KEY", "server-key")])
        .validate_environment(headers, None)
        .expect("the environment key is set");

    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer server-key")
    );
    assert_eq!(validated.get("x-extra").map(String::as_str), Some("kept"));
}

#[test]
fn an_explicit_key_beats_the_environment_key_for_both_configurations() {
    let environment = [("RECRAFT_API_KEY", "server-key")];
    let explicit = SecretString::from("caller-key");
    let empty = SecretString::from("");

    let generated = generation(&environment)
        .validate_environment(ImageHeaders::new(), Some(&explicit))
        .expect("the explicit key is set");
    let edited = edit(&environment)
        .validate_environment(ImageHeaders::new(), Some(&explicit))
        .expect("the explicit key is set");
    assert_eq!(
        generated.get("Authorization").map(String::as_str),
        Some("Bearer caller-key")
    );
    assert_eq!(
        edited.get("Authorization").map(String::as_str),
        Some("Bearer caller-key")
    );
    let from_environment = edit(&environment)
        .validate_environment(ImageHeaders::new(), Some(&empty))
        .expect("an empty explicit key falls through");
    assert_eq!(
        from_environment.get("Authorization").map(String::as_str),
        Some("Bearer server-key")
    );
}

#[test]
fn validate_environment_fails_without_a_key() {
    assert_eq!(
        generation(&[])
            .validate_environment(ImageHeaders::new(), None)
            .expect_err("a missing key must fail"),
        RecraftError::MissingApiKey
    );
    assert_eq!(
        edit(&[("RECRAFT_API_KEY", "true")])
            .validate_environment(ImageHeaders::new(), None)
            .expect_err("a boolean environment value reads as unset"),
        RecraftError::MissingApiKey
    );
}

#[rstest]
#[case::dropping(true)]
#[case::keeping(false)]
fn edit_mapping_filters_to_three_names_whatever_drop_params_says(#[case] drop_params: bool) {
    let options = params(&[
        ("n", integer(2)),
        ("quality", text("hd")),
        ("style", text("realistic_image")),
        ("response_format", text("b64_json")),
        ("size", text("1024x1024")),
    ]);

    let mapped = RecraftImageEditConfig::map_openai_params(options, "model", drop_params);

    assert_eq!(
        mapped,
        params(&[
            ("n", integer(2)),
            ("style", text("realistic_image")),
            ("response_format", text("b64_json")),
        ])
    );
}

#[test]
fn the_edit_request_pops_strength_and_defaults_it() {
    let (supplied, _) = RecraftImageEditConfig::transform_image_edit_request(
        "recraft/recraftv3",
        None,
        None,
        params(&[("strength", integer(3))]),
    );
    let (defaulted, _) = RecraftImageEditConfig::transform_image_edit_request(
        "recraft/recraftv3",
        None,
        None,
        ImageParams::new(),
    );
    let (explicit_null, _) = RecraftImageEditConfig::transform_image_edit_request(
        "recraft/recraftv3",
        None,
        None,
        params(&[("strength", Json::Null)]),
    );

    assert_eq!(
        serialized(&supplied),
        json(r#"{"model":"recraft/recraftv3","strength":3}"#)
    );
    assert_eq!(
        serialized(&defaulted),
        json(r#"{"model":"recraft/recraftv3","strength":0.2}"#)
    );
    assert_eq!(
        serialized(&explicit_null),
        json(r#"{"model":"recraft/recraftv3","strength":null}"#)
    );
}

#[test]
fn the_edit_request_keeps_provider_fields_and_drops_image() {
    let options = params(&[
        ("strength", Json::Null),
        ("style_id", text("style-1")),
        ("style", text("realistic_image")),
        ("substyle", text("digital_illustration")),
        ("response_format", text("url")),
        ("negative_prompt", text("blurry")),
        ("controls", json(r#"{"colors":[]}"#)),
        ("image", text("not-a-file")),
    ]);

    let (request, files) = RecraftImageEditConfig::transform_image_edit_request(
        "recraft/recraftv3",
        None,
        None,
        options,
    );

    assert_eq!(
        serialized(&request),
        json(
            r#"{"model":"recraft/recraftv3","strength":null,"style_id":"style-1","style":"realistic_image","substyle":"digital_illustration","response_format":"url","negative_prompt":"blurry","controls":{"colors":[]}}"#
        )
    );
    assert!(files.is_empty());
}

#[rstest]
#[case::argument(
    Some("a cat"),
    ImageParams::new(),
    r#"{"model":"recraft/recraftv3","strength":0.2,"prompt":"a cat"}"#
)]
#[case::from_options(
    None,
    params(&[("prompt", text("from options"))]),
    r#"{"model":"recraft/recraftv3","strength":0.2,"prompt":"from options"}"#
)]
#[case::argument_wins(
    Some("a cat"),
    params(&[("prompt", text("from options"))]),
    r#"{"model":"recraft/recraftv3","strength":0.2,"prompt":"a cat"}"#
)]
#[case::null_is_absent(
    None,
    params(&[("prompt", Json::Null)]),
    r#"{"model":"recraft/recraftv3","strength":0.2}"#
)]
fn the_edit_request_includes_prompt_only_when_it_has_a_value(
    #[case] prompt: Option<&str>,
    #[case] options: ImageParams,
    #[case] expected: &str,
) {
    let (request, _) = RecraftImageEditConfig::transform_image_edit_request(
        "recraft/recraftv3",
        prompt,
        None,
        options,
    );

    assert_eq!(serialized(&request), json(expected));
}

#[test]
fn the_edit_request_takes_the_first_image_only() {
    let first = ImageFile {
        filename: Some("cat.jpg".to_owned()),
        content: JPEG.to_vec(),
        content_type: None,
    };
    let second = ImageFile {
        filename: Some("dog.png".to_owned()),
        content: PNG.to_vec(),
        content_type: None,
    };

    let (_, files) = RecraftImageEditConfig::transform_image_edit_request(
        "model",
        None,
        Some(&[first, second]),
        ImageParams::new(),
    );

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, "image");
    assert_eq!(files[0].1.filename.as_deref(), Some("cat.jpg"));
    assert_eq!(files[0].1.content_type.as_deref(), Some("image/jpeg"));
    assert_eq!(files[0].1.content.as_slice(), JPEG);
}

#[test]
fn an_unnamed_image_uploads_as_image_png_with_a_sniffed_mime() {
    let png = ImageFile {
        filename: None,
        content: PNG.to_vec(),
        content_type: Some("application/octet-stream".to_owned()),
    };
    let unknown = ImageFile {
        filename: None,
        content: b"plain text".to_vec(),
        content_type: None,
    };

    let (_, png_files) = RecraftImageEditConfig::transform_image_edit_request(
        "model",
        None,
        Some(&[png]),
        ImageParams::new(),
    );
    let (_, unknown_files) = RecraftImageEditConfig::transform_image_edit_request(
        "model",
        None,
        Some(&[unknown]),
        ImageParams::new(),
    );

    assert_eq!(png_files[0].1.filename.as_deref(), Some("image.png"));
    assert_eq!(png_files[0].1.content_type.as_deref(), Some("image/png"));
    assert_eq!(unknown_files[0].1.filename.as_deref(), Some("image.png"));
    assert_eq!(
        unknown_files[0].1.content_type.as_deref(),
        Some("image/png")
    );
}

#[test]
fn the_edit_request_adds_no_part_without_an_image() {
    let (_, absent) = RecraftImageEditConfig::transform_image_edit_request(
        "model",
        None,
        None,
        ImageParams::new(),
    );
    let (_, empty) = RecraftImageEditConfig::transform_image_edit_request(
        "model",
        None,
        Some(&[]),
        ImageParams::new(),
    );

    assert!(absent.is_empty());
    assert!(empty.is_empty());
}

#[test]
fn generation_appends_to_the_existing_response() {
    let existing = ImageData {
        b64_json: Some("existing".to_owned()),
        url: None,
        revised_prompt: None,
    };
    let mut response = ImageResponse {
        created: 7,
        data: Some(vec![existing.clone()]),
        usage: None,
    };

    RecraftImageGenerationConfig::transform_image_generation_response(
        &upstream(
            200,
            r#"{"data":[{"url":"u1"},{"b64_json":"b1","url":null}]}"#,
        ),
        &mut response,
    )
    .expect("the reply decodes");

    assert_eq!(response.created, 7);
    assert_eq!(
        response.data,
        Some(vec![
            existing,
            ImageData {
                b64_json: None,
                url: Some("u1".to_owned()),
                revised_prompt: None,
            },
            ImageData {
                b64_json: Some("b1".to_owned()),
                url: None,
                revised_prompt: None,
            },
        ])
    );
}

#[test]
fn generation_fills_absent_data_even_from_an_empty_reply() {
    let mut response = ImageResponse::default();

    RecraftImageGenerationConfig::transform_image_generation_response(
        &upstream(200, r#"{"data":[]}"#),
        &mut response,
    )
    .expect("an empty list succeeds");

    assert_eq!(response.data, Some(Vec::new()));
}

#[test]
fn the_edit_response_starts_fresh_and_keeps_the_caller_created() {
    let response = RecraftImageEditConfig::transform_image_edit_response(
        &upstream(200, r#"{"data":[{"url":"u1"},{"b64_json":"b1"}]}"#),
        1_700_000_000,
    )
    .expect("the reply decodes");

    assert_eq!(response.created, 1_700_000_000);
    assert_eq!(response.usage, Some(ImageUsage::default()));
    assert_eq!(
        response.data,
        Some(vec![
            ImageData {
                b64_json: None,
                url: Some("u1".to_owned()),
                revised_prompt: None,
            },
            ImageData {
                b64_json: Some("b1".to_owned()),
                url: None,
                revised_prompt: None,
            },
        ])
    );
}

#[rstest]
#[case::not_an_object("[]")]
#[case::missing_data("{}")]
#[case::null_data(r#"{"data":null}"#)]
#[case::data_not_a_list(r#"{"data":{}}"#)]
#[case::image_not_an_object(r#"{"data":[1]}"#)]
#[case::url_not_text(r#"{"data":[{"url":5}]}"#)]
#[case::b64_json_not_text(r#"{"data":[{"b64_json":true}]}"#)]
fn structural_reply_failures_name_the_shape(#[case] body: &str) {
    let error = decode_recraft_images(&upstream(200, body)).expect_err("the shape is rejected");

    assert!(error.malformed().is_none());
}

#[test]
fn structural_reply_failures_are_distinct() {
    assert!(matches!(
        decode_recraft_images(&upstream(200, "[]")),
        Err(RecraftReplyError::NotObject)
    ));
    assert!(matches!(
        decode_recraft_images(&upstream(200, "{}")),
        Err(RecraftReplyError::MissingData)
    ));
    assert!(matches!(
        decode_recraft_images(&upstream(200, r#"{"data":null}"#)),
        Err(RecraftReplyError::DataShape)
    ));
    assert!(matches!(
        decode_recraft_images(&upstream(200, r#"{"data":[1]}"#)),
        Err(RecraftReplyError::ImageShape)
    ));
    assert!(matches!(
        decode_recraft_images(&upstream(200, r#"{"data":[{"url":5}]}"#)),
        Err(RecraftReplyError::ImageField { field: "url" })
    ));
    assert!(matches!(
        decode_recraft_images(&upstream(200, r#"{"data":[{"b64_json":true}]}"#)),
        Err(RecraftReplyError::ImageField { field: "b64_json" })
    ));
}

#[test]
fn an_unparsable_reply_keeps_the_upstream_status_and_headers() {
    let error =
        decode_recraft_images(&upstream(502, "not json")).expect_err("the body is not JSON");

    assert!(matches!(error, RecraftReplyError::Malformed { .. }));
    let (status, headers) = error.malformed().expect("the failure keeps the reply");
    assert_eq!(status, 502);
    assert_eq!(headers.get("x-request-id").map(String::as_str), Some("req"));
}

#[test]
fn the_generation_selector_always_answers_the_same_configuration() {
    let environment = [
        ("RECRAFT_API_KEY", "server-key"),
        ("RECRAFT_API_BASE", "https://env.test"),
    ];

    let first = get_recraft_image_generation_config("recraft/recraftv3", &lookup(&environment));
    let second = get_recraft_image_generation_config("something-else", &lookup(&environment));

    assert_eq!(first.settings.api_base, "https://env.test");
    assert_eq!(second.settings.api_base, "https://env.test");
    assert_eq!(
        second
            .settings
            .api_key
            .map(|key| key.expose_secret().to_owned()),
        Some("server-key".to_owned())
    );
}

#[test]
fn cost_multiplies_the_returned_count_by_the_resolved_rate() {
    let image = billing(2);

    let cost = cost_calculator(
        "recraft/oracle-unknown",
        ImageResponseView::Image(&image),
        Some(&rate(0.25)),
        &NoPrices,
    )
    .expect("the model info resolves");

    assert_eq!(cost, Decimal::new(5, 1));
}

#[rstest]
#[case::empty_count(0, Some(0.25))]
#[case::missing_rate(2, None)]
fn cost_uses_the_rate_the_table_gives(#[case] image_count: i64, #[case] per_image: Option<f64>) {
    let image = billing(image_count);
    let table = per_image.map_or_else(PriceRecord::new, rate);

    let cost = cost_calculator(
        "recraft/oracle-unknown",
        ImageResponseView::Image(&image),
        Some(&table),
        &NoPrices,
    )
    .expect("the model info resolves");

    assert_eq!(cost, Decimal::ZERO);
}

#[rstest]
#[case::false_bool(Scalar::Bool(false))]
#[case::null(Scalar::Null)]
#[case::array(Scalar::Array(Vec::new()))]
fn a_falsey_rate_costs_nothing(#[case] value: Scalar) {
    let image = billing(3);
    let table = PriceRecord::from([("output_cost_per_image".to_owned(), value)]);

    let cost = cost_calculator(
        "recraft/oracle-unknown",
        ImageResponseView::Image(&image),
        Some(&table),
        &NoPrices,
    )
    .expect("the model info resolves");

    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn cost_rejects_a_non_image_response_after_resolving_the_model() {
    let error = cost_calculator(
        "recraft/oracle-unknown",
        ImageResponseView::Other,
        Some(&rate(0.25)),
        &NoPrices,
    )
    .expect_err("the response is not an image response");

    assert!(matches!(error, ImageCostError::NotImageResponse));
    let unresolved = cost_calculator(
        "recraft/oracle-unknown",
        ImageResponseView::Other,
        None,
        &NoPrices,
    )
    .expect_err("the model has no price entry");
    assert!(matches!(
        unresolved,
        ImageCostError::UnknownModel { model } if model == "recraft/oracle-unknown"
    ));
}

#[rstest]
fn replays_the_recraft_images_corpus(#[files("fuzz/corpus/recraft_images/*")] path: PathBuf) {
    let body = fs::read(path).expect("the corpus seed is read");
    let response = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body,
    };

    drop(decode_recraft_images(&response));
}
