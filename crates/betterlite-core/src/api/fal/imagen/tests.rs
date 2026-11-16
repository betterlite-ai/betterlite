#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use rstest::rstest;
use serde_json::Number;

use super::size::map_aspect_ratio;
use super::{
    ENDPOINT, ImagenImageResponse, SUPPORTED_OPENAI_PARAMS, map_openai_params, project_response,
};
use crate::api::fal::images::decode::{FalImage, MAX_FAL_REPLY_BYTES};
use crate::api::fal::images::error::FalImageError;
use crate::api::fal::images::params::image_generation_body;
use crate::api::fal::images::settings::FalImageSettings;

const MODEL: &str = "fal-ai/imagen4/preview/ultra";

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn params(entries: &[(&str, Json)]) -> ImageParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn mapped(
    non_default: &[(&str, Json)],
    optional: &[(&str, Json)],
) -> Result<ImageParams, FalImageError> {
    map_openai_params(&params(non_default), params(optional), MODEL, false)
}

fn headers() -> ImageHeaders {
    ImageHeaders::from([("x-request-id".to_owned(), "req-1".to_owned())])
}

fn projected(body: &str) -> ImagenImageResponse {
    let mut response = ImagenImageResponse::default();
    project_response(200, &headers(), body.as_bytes(), &mut response).expect("a projected reply");
    response
}

fn url_image(url: Option<&str>) -> FalImage {
    FalImage {
        url: url.map(str::to_owned),
        b64_json: None,
        provider_specific_fields: None,
    }
}

#[test]
fn imagen_uses_one_preview_endpoint_for_every_variant() {
    let settings = FalImageSettings::default();
    assert_eq!(ENDPOINT, "fal-ai/imagen4/preview");
    assert_eq!(
        settings.complete_url(None, ENDPOINT),
        "https://fal.run/fal-ai/imagen4/preview"
    );
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["n", "response_format", "size"]);
}

#[rstest]
#[case::square_hd("1024x1024", "1:1")]
#[case::square("512x512", "1:1")]
#[case::landscape("1792x1024", "16:9")]
#[case::portrait("1024x1792", "9:16")]
#[case::landscape_four_three("1024x768", "4:3")]
#[case::portrait_three_four("768x1024", "3:4")]
#[case::square_lower_edge("19x20", "1:1")]
#[case::square_upper_edge("21x20", "1:1")]
#[case::below_square_band_is_gap("9x10", "1:1")]
#[case::above_square_band_is_gap("11x10", "1:1")]
#[case::wide_edge("17x10", "16:9")]
#[case::just_under_wide("169x100", "4:3")]
#[case::wider_than_wide("5x1", "16:9")]
#[case::tall_edge("3x5", "9:16")]
#[case::just_over_tall("61x100", "3:4")]
#[case::zero_width("0x100", "9:16")]
#[case::four_three_edge("6x5", "4:3")]
#[case::just_under_four_three("119x100", "1:1")]
#[case::three_four_edge("4x5", "3:4")]
#[case::just_over_three_four("81x100", "1:1")]
#[case::negative_height("1024x-768", "9:16")]
#[case::both_negative("-1024x-768", "4:3")]
#[case::zero_height("100x0", "1:1")]
#[case::python_int_spacing("1792 x 1024", "16:9")]
#[case::python_int_underscores("1_7x1_0", "16:9")]
#[case::not_a_size("auto", "1:1")]
#[case::empty("", "1:1")]
#[case::missing_height("10x", "1:1")]
#[case::three_parts("1x2x3", "1:1")]
#[case::fal_separator("1792-x-1024", "1:1")]
#[case::decimal_part("1.5x2", "1:1")]
#[case::uppercase_separator("1792X1024", "1:1")]
fn size_selects_a_ratio_by_thresholds(#[case] size: &str, #[case] ratio: &str) {
    assert_eq!(map_aspect_ratio(size), ratio);
}

#[test]
fn imagen_maps_n_and_size_and_ignores_response_format() {
    assert_eq!(
        mapped(
            &[
                ("n", number(3)),
                ("response_format", text("b64_json")),
                ("size", text("1792x1024")),
            ],
            &[]
        )
        .expect("mapped"),
        params(&[("num_images", number(3)), ("aspect_ratio", text("16:9"))])
    );
}

#[test]
fn imagen_response_format_is_ignored_even_with_a_url_value() {
    assert_eq!(
        mapped(&[("response_format", text("url"))], &[]).expect("mapped"),
        ImageParams::new()
    );
}

#[rstest]
#[case::n_in_options(&[("n", number(2))], &[("n", number(5))], &[("n", number(5))])]
#[case::translated_key_does_not_protect_n(
    &[("n", number(2))],
    &[("num_images", number(1))],
    &[("num_images", number(2))]
)]
#[case::size_in_options(&[("size", text("1024x768"))], &[("size", text("kept"))], &[("size", text("kept"))])]
#[case::aspect_ratio_does_not_protect_size(
    &[("size", text("1024x768"))],
    &[("aspect_ratio", text("kept"))],
    &[("aspect_ratio", text("4:3"))]
)]
#[case::unsupported_key_in_options_is_skipped(&[("seed", number(1))], &[("seed", number(7))], &[("seed", number(7))])]
fn original_key_in_the_options_decides_precedence(
    #[case] non_default: &[(&str, Json)],
    #[case] optional: &[(&str, Json)],
    #[case] expected: &[(&str, Json)],
) {
    assert_eq!(
        mapped(non_default, optional).expect("mapped"),
        params(expected)
    );
}

#[test]
fn imagen_later_unsupported_key_named_like_a_translation_is_skipped() {
    assert_eq!(
        mapped(&[("n", number(2)), ("num_images", number(9))], &[]).expect("mapped"),
        params(&[("num_images", number(2))])
    );
}

#[test]
fn imagen_unsupported_parameter_fails_unless_dropped() {
    let error = mapped(&[("seed", number(1))], &[]).expect_err("unsupported");
    assert!(matches!(
        error,
        FalImageError::UnsupportedParameters { parameters, model }
            if parameters == ["seed"] && model == MODEL
    ));
    assert_eq!(
        map_openai_params(
            &params(&[("seed", number(1))]),
            ImageParams::new(),
            MODEL,
            true
        )
        .expect("dropped"),
        ImageParams::new()
    );
}

#[test]
fn imagen_size_must_be_text() {
    let error = mapped(&[("size", number(1024))], &[]).expect_err("not text");
    assert!(matches!(error, FalImageError::SizeNotText));
}

#[test]
fn imagen_request_body_overlays_options_on_the_prompt() {
    let body = image_generation_body(
        "a cat",
        mapped(&[("n", number(2))], &[("prompt", text("a dog"))]).expect("mapped"),
    );
    assert_eq!(
        body,
        params(&[("prompt", text("a dog")), ("num_images", number(2))])
    );
}

#[test]
fn imagen_reply_keeps_urls_only_and_discards_base64_and_dimensions() {
    let response = projected(
        r#"{"images": ["https://a.test/1.png", {"url": "https://b.test/2.png", "b64_json": "aGk=", "width": 640, "height": 480, "content_type": "image/webp"}, {"b64_json": "aGk="}, {"url": null}, 7, null, true, ["x"]], "seed": 42, "timings": {"inference": 1.5}, "has_nsfw_concepts": [false]}"#,
    );
    assert_eq!(
        response.data,
        vec![
            url_image(Some("https://a.test/1.png")),
            url_image(Some("https://b.test/2.png")),
            url_image(None),
            url_image(None),
        ]
    );
    assert_eq!(response.hidden_params, params(&[("seed", number(42))]));
}

#[rstest]
#[case::null_seed(r#"{"images": [], "seed": null}"#, &[("seed", Json::Null)])]
#[case::zero_seed(r#"{"images": [], "seed": 0}"#, &[("seed", number(0))])]
#[case::absent_seed(r#"{"images": []}"#, &[])]
fn imagen_seed_is_copied_by_presence(#[case] body: &str, #[case] hidden: &[(&str, Json)]) {
    assert_eq!(projected(body).hidden_params, params(hidden));
}

#[rstest]
#[case::images_object(r#"{"images": {"url": "https://a.test/1.png"}, "seed": 1}"#)]
#[case::images_text(r#"{"images": "https://a.test/1.png"}"#)]
#[case::images_missing("{}")]
fn imagen_reply_without_an_image_list_adds_nothing(#[case] body: &str) {
    assert!(projected(body).data.is_empty());
}

#[test]
fn imagen_reply_appends_to_existing_images() {
    let mut response = ImagenImageResponse {
        data: vec![url_image(Some("https://old.test/0.png"))],
        hidden_params: params(&[("seed", number(1))]),
    };
    project_response(
        200,
        &headers(),
        br#"{"images": ["https://a.test/1.png"]}"#,
        &mut response,
    )
    .expect("projected");
    assert_eq!(
        response.data,
        vec![
            url_image(Some("https://old.test/0.png")),
            url_image(Some("https://a.test/1.png")),
        ]
    );
    assert_eq!(response.hidden_params, params(&[("seed", number(1))]));
}

#[test]
fn imagen_non_text_url_names_the_field() {
    let mut response = ImagenImageResponse::default();
    let error = project_response(
        200,
        &headers(),
        br#"{"images": [{"url": 5}]}"#,
        &mut response,
    )
    .expect_err("url is not text");
    assert!(matches!(
        error,
        FalImageError::FieldNotText {
            field: "url",
            model: ENDPOINT
        }
    ));
}

#[rstest]
#[case::not_json("{", 502)]
#[case::not_an_object("[]", 200)]
fn imagen_bad_reply_carries_its_status(#[case] body: &str, #[case] status: u16) {
    let mut response = ImagenImageResponse::default();
    let error = project_response(status, &headers(), body.as_bytes(), &mut response)
        .expect_err("bad reply");
    assert!(matches!(
        error,
        FalImageError::ReplyDecode { status: seen, .. }
            | FalImageError::ReplyNotAnObject { status: seen, .. }
            if seen == status
    ));
}

#[test]
fn imagen_oversized_reply_is_refused() {
    let mut response = ImagenImageResponse::default();
    let body = vec![b' '; MAX_FAL_REPLY_BYTES + 1];
    let error = project_response(200, &headers(), &body, &mut response).expect_err("too large");
    assert!(matches!(error, FalImageError::ReplyTooLarge { .. }));
}
