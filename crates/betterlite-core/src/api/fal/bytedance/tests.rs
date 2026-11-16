#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::{BytedanceModel, MAPPING_PROFILE};
use crate::api::fal::flux::{FluxError, FluxImageResponse, project_response};
use crate::api::fal::images::decode::{FalImage, FalImageProviderSpecificFields};
use crate::api::fal::images::params::image_generation_body;
use crate::api::fal::images::settings::FalImageSettings;

const MODEL: &str = "fal-ai/bytedance/seedream/v3/text-to-image";

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

fn dimensions(width: i64, height: i64) -> Json {
    Json::Object(IndexMap::from([
        ("width".to_owned(), number(width)),
        ("height".to_owned(), number(height)),
    ]))
}

fn mapped(
    non_default: &[(&str, Json)],
    optional: &[(&str, Json)],
) -> Result<ImageParams, FluxError> {
    MAPPING_PROFILE.map_openai_params(&params(non_default), params(optional), MODEL, false)
}

fn image_size_for(size: &Json) -> Json {
    mapped(&[("size", size.clone())], &[])
        .expect("mapped")
        .get("image_size")
        .cloned()
        .expect("an image size")
}

#[test]
fn bytedance_models_use_their_fixed_endpoints() {
    let settings = FalImageSettings::default();
    assert_eq!(
        BytedanceModel::SeedreamV3.endpoint(),
        "fal-ai/bytedance/seedream/v3/text-to-image"
    );
    assert_eq!(
        BytedanceModel::DreaminaV31.endpoint(),
        "fal-ai/bytedance/dreamina/v3.1/text-to-image"
    );
    assert_eq!(
        settings.complete_url(None, BytedanceModel::DreaminaV31.endpoint()),
        "https://fal.run/fal-ai/bytedance/dreamina/v3.1/text-to-image"
    );
}

#[rstest]
#[case::square_hd("1024x1024", text("square_hd"))]
#[case::square("512x512", text("square"))]
#[case::landscape_sixteen_nine("1792x1024", text("landscape_16_9"))]
#[case::portrait_sixteen_nine("1024x1792", text("portrait_16_9"))]
#[case::landscape_four_three("1024x768", text("landscape_4_3"))]
#[case::portrait_four_three("768x1024", text("portrait_4_3"))]
#[case::custom_dimensions("1500x1000", dimensions(1500, 1000))]
#[case::zero_height_is_kept("1024x0", dimensions(1024, 0))]
#[case::negative_width_is_kept("-5x6", dimensions(-5, 6))]
#[case::python_int_underscores("1_0x2_0", dimensions(10, 20))]
#[case::no_separator("auto", text("landscape_4_3"))]
#[case::empty("", text("landscape_4_3"))]
#[case::missing_height("10x", text("landscape_4_3"))]
#[case::three_parts("1x2x3", text("landscape_4_3"))]
#[case::fal_separator("1024-x-768", text("landscape_4_3"))]
#[case::uppercase_separator("1024X768", text("landscape_4_3"))]
#[case::min_i64("-9223372036854775808x1", text("landscape_4_3"))]
#[case::non_ascii_digits(
    "\u{661}\u{660}\u{662}\u{664}x\u{667}\u{666}\u{668}",
    text("landscape_4_3")
)]
fn text_size_maps_to_a_preset_or_custom_dimensions(#[case] size: &str, #[case] expected: Json) {
    assert_eq!(image_size_for(&text(size)), expected);
}

#[rstest]
#[case::object(dimensions(512, 512))]
#[case::partial_object(Json::Object(IndexMap::from([("width".to_owned(), number(512))])))]
#[case::number(number(7))]
#[case::boolean(Json::Bool(true))]
#[case::null(Json::Null)]
fn non_text_size_passes_through(#[case] size: Json) {
    assert_eq!(image_size_for(&size), size);
}

#[rstest]
#[case::url("url", "jpeg")]
#[case::b64_json("b64_json", "jpeg")]
#[case::other("png", "png")]
fn response_format_maps_to_output_format(#[case] requested: &str, #[case] expected: &str) {
    assert_eq!(
        mapped(&[("response_format", text(requested))], &[]).expect("mapped"),
        params(&[("output_format", text(expected))])
    );
}

#[test]
fn response_format_that_is_not_text_passes_through() {
    assert_eq!(
        mapped(&[("response_format", number(1))], &[]).expect("mapped"),
        params(&[("output_format", number(1))])
    );
}

#[rstest]
#[case::n_maps(&[("n", number(2))], &[], &[("num_images", number(2))])]
#[case::n_overwrites_the_translated_key(
    &[("n", number(2))],
    &[("num_images", number(1))],
    &[("num_images", number(2))]
)]
#[case::original_n_key_wins(&[("n", number(2))], &[("n", number(5))], &[("n", number(5))])]
#[case::original_size_key_wins(
    &[("size", text("1024x768"))],
    &[("size", text("kept"))],
    &[("size", text("kept"))]
)]
#[case::original_response_format_key_wins(
    &[("response_format", text("url"))],
    &[("response_format", text("png"))],
    &[("response_format", text("png"))]
)]
#[case::translated_size_key_does_not_protect(
    &[("size", text("1024x768"))],
    &[("image_size", text("kept"))],
    &[("image_size", text("landscape_4_3"))]
)]
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
fn bytedance_unsupported_parameter_fails_unless_dropped() {
    let error = mapped(&[("seed", number(1))], &[]).expect_err("unsupported");
    assert!(matches!(
        error,
        FluxError::UnsupportedParameter { parameter, model }
            if parameter == "seed" && model == MODEL
    ));
    assert_eq!(
        MAPPING_PROFILE
            .map_openai_params(
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
fn bytedance_request_body_overlays_options_on_the_prompt() {
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
fn bytedance_reply_keeps_flux_metadata_and_image_fields() {
    let mut response = FluxImageResponse::default();
    project_response(
        200,
        &ImageHeaders::new(),
        br#"{"images": [{"url": "https://b.test/2.png", "b64_json": "aGk=", "width": 640, "height": 480, "content_type": "image/webp"}], "seed": null, "timings": {"inference": 2}, "has_nsfw_concepts": [false], "prompt": "ignored"}"#,
        &mut response,
    )
    .expect("projected");
    assert_eq!(
        response.data,
        vec![FalImage {
            url: Some("https://b.test/2.png".to_owned()),
            b64_json: Some("aGk=".to_owned()),
            provider_specific_fields: Some(FalImageProviderSpecificFields {
                width: Some(640),
                height: Some(480),
                content_type: Some("image/webp".to_owned()),
            }),
        }]
    );
    assert_eq!(
        response.hidden_params,
        params(&[
            ("seed", Json::Null),
            (
                "timings",
                Json::Object(IndexMap::from([("inference".to_owned(), number(2),)]))
            ),
            ("has_nsfw_concepts", Json::List(vec![Json::Bool(false)])),
        ])
    );
}
