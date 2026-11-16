#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::{SUPPORTED_OPENAI_PARAMS, endpoint, map_openai_params};
use crate::api::fal::imagen;
use crate::api::fal::images::decode::{
    FalImage, FalImageProviderSpecificFields, append_generation_images,
};
use crate::api::fal::images::error::FalImageError;
use crate::api::fal::images::params::image_generation_body;
use crate::api::fal::images::settings::FalImageSettings;

const MODEL: &str = "fal-ai/nano-banana-pro";

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

fn aspect_ratio_for(size: &Json) -> Json {
    mapped(&[("size", size.clone())], &[])
        .expect("mapped")
        .get("aspect_ratio")
        .cloned()
        .expect("an aspect ratio")
}

#[rstest]
#[case::bare_name("nano-banana", "fal-ai/nano-banana")]
#[case::prefixed("fal-ai/nano-banana-2", "fal-ai/nano-banana-2")]
#[case::gemini("gemini-25-flash-image", "fal-ai/gemini-25-flash-image")]
#[case::other_namespace("other/nano-banana", "fal-ai/other/nano-banana")]
#[case::prefix_without_slash("fal-ai", "fal-ai/fal-ai")]
fn endpoint_prepends_the_prefix_only_when_missing(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(endpoint(model), expected);
}

#[test]
fn complete_url_joins_the_base_and_the_requested_endpoint() {
    let settings = FalImageSettings::default();
    assert_eq!(
        settings.complete_url(Some("https://fal.test///"), &endpoint("nano-banana")),
        "https://fal.test/fal-ai/nano-banana"
    );
    assert_eq!(
        settings.complete_url(None, &endpoint("fal-ai/nano-banana-pro")),
        "https://fal.run/fal-ai/nano-banana-pro"
    );
}

#[test]
fn nano_banana_advertises_the_three_openai_params() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["n", "response_format", "size"]);
}

#[rstest]
#[case::square_hd("1024x1024", "1:1")]
#[case::landscape("1792x1024", "16:9")]
#[case::portrait("1024x1792", "9:16")]
#[case::four_three("1024x768", "4:3")]
#[case::three_four("768x1024", "3:4")]
#[case::three_two("1536x1024", "3:2")]
#[case::two_three("1024x1536", "2:3")]
#[case::ultra_wide("2048x876", "21:9")]
#[case::wider_than_any("10x1", "21:9")]
#[case::five_four("5x4", "5:4")]
#[case::four_five("4x5", "4:5")]
#[case::tie_between_one_and_five_four_keeps_the_first("9x8", "5:4")]
#[case::tie_between_four_five_and_one_keeps_the_first("9x10", "1:1")]
#[case::tie_between_four_five_and_three_four_keeps_the_first("31x40", "4:5")]
#[case::exact_tie_between_wide_ratios_takes_the_first("37x18", "21:9")]
#[case::narrower_than_any("1x10", "9:16")]
#[case::zero_width("0x100", "9:16")]
#[case::negative_height_is_the_smallest_ratio("1024x-768", "9:16")]
#[case::both_negative("-1024x-768", "4:3")]
#[case::python_int_spacing(" 1792 x 1024 ", "16:9")]
#[case::python_int_underscores("1_7x1_0", "16:9")]
fn size_selects_the_nearest_ratio_by_distance(#[case] size: &str, #[case] ratio: &str) {
    assert_eq!(aspect_ratio_for(&text(size)), text(ratio));
}

#[rstest]
#[case::zero_height(text("100x0"))]
#[case::no_separator(text("auto"))]
#[case::empty(text(""))]
#[case::missing_height(text("10x"))]
#[case::missing_width(text("x10"))]
#[case::three_parts(text("1x2x3"))]
#[case::fal_separator(text("1792-x-1024"))]
#[case::decimal_part(text("1.5x2"))]
#[case::uppercase_separator(text("1792X1024"))]
#[case::min_i64(text("-9223372036854775808x1"))]
#[case::non_ascii_digits(text("\u{661}\u{660}\u{662}\u{664}x\u{667}\u{666}\u{668}"))]
#[case::number(number(1024))]
#[case::boolean(Json::Bool(true))]
#[case::null(Json::Null)]
#[case::object(Json::Object(IndexMap::from([("width".to_owned(), number(5))])))]
fn invalid_sizes_select_one_to_one(#[case] size: Json) {
    assert_eq!(aspect_ratio_for(&size), text("1:1"));
}

#[test]
fn nano_banana_ignores_response_format_even_when_it_is_in_the_options() {
    assert_eq!(
        mapped(
            &[("response_format", text("b64_json"))],
            &[("response_format", text("kept"))]
        )
        .expect("mapped"),
        params(&[("response_format", text("kept"))])
    );
    assert_eq!(
        mapped(&[("response_format", text("url"))], &[]).expect("mapped"),
        ImageParams::new()
    );
}

#[rstest]
#[case::n_maps(&[("n", number(2))], &[], &[("num_images", number(2))])]
#[case::existing_num_images_wins(
    &[("n", number(2))],
    &[("num_images", number(1))],
    &[("num_images", number(1))]
)]
#[case::original_n_key_does_not_protect(
    &[("n", number(2))],
    &[("n", number(5))],
    &[("n", number(5)), ("num_images", number(2))]
)]
#[case::existing_aspect_ratio_wins_over_size(
    &[("size", text("1792x1024"))],
    &[("aspect_ratio", text("9:21"))],
    &[("aspect_ratio", text("9:21"))]
)]
#[case::original_size_key_does_not_protect(
    &[("size", text("1792x1024"))],
    &[("size", text("kept"))],
    &[("size", text("kept")), ("aspect_ratio", text("16:9"))]
)]
#[case::resolution_is_untouched(
    &[("n", number(1)), ("size", text("1024x1024"))],
    &[("resolution", text("4K"))],
    &[("resolution", text("4K")), ("num_images", number(1)), ("aspect_ratio", text("1:1"))]
)]
#[case::no_resolution_default(&[], &[], &[])]
fn translated_destination_keys_decide_precedence(
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
fn nano_banana_unsupported_parameter_fails_unless_dropped_or_already_set() {
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
    assert_eq!(
        mapped(&[("seed", number(1))], &[("seed", number(7))]).expect("kept"),
        params(&[("seed", number(7))])
    );
}

#[test]
fn nano_banana_later_unsupported_key_named_like_a_translation_is_skipped() {
    assert_eq!(
        mapped(&[("n", number(2)), ("num_images", number(9))], &[]).expect("mapped"),
        params(&[("num_images", number(2))])
    );
}

#[test]
fn nano_banana_request_body_overlays_options_on_the_prompt() {
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
fn nano_banana_reply_keeps_base64_and_dimensions_unlike_imagen() {
    let headers = ImageHeaders::new();
    let body = br#"{"images": [{"url": "https://b.test/2.png", "b64_json": "aGk=", "width": 640, "height": 480, "content_type": "image/webp"}], "seed": 42}"#;
    let mut nano = None;
    append_generation_images(&mut nano, 200, &headers, body).expect("converted");
    assert_eq!(
        nano,
        Some(vec![FalImage {
            url: Some("https://b.test/2.png".to_owned()),
            b64_json: Some("aGk=".to_owned()),
            provider_specific_fields: Some(FalImageProviderSpecificFields {
                width: Some(640),
                height: Some(480),
                content_type: Some("image/webp".to_owned()),
            }),
        }])
    );
    let mut imagen_response = imagen::ImagenImageResponse::default();
    imagen::project_response(200, &headers, body, &mut imagen_response).expect("projected");
    assert_eq!(
        imagen_response.data,
        vec![FalImage {
            url: Some("https://b.test/2.png".to_owned()),
            b64_json: None,
            provider_specific_fields: None,
        }]
    );
}
