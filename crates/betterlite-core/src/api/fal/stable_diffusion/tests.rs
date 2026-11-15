#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use indexmap::IndexMap;
use rstest::rstest;

use super::complete_url;
use super::error::StableDiffusionError;
use super::params::map_openai_params;
use super::reply::transform_reply;
use super::size::{FALLBACK_SIZE, map_image_size};
use crate::api::fal::images::decode::MAX_FAL_REPLY_BYTES;
use crate::api::fal::images::error::FalImageError;
use crate::api::fal::images::params::image_generation_body;
use crate::api::fal::images::settings::{DEFAULT_BASE_URL, FalImageSettings};

const MODEL: &str = "stable-diffusion-v35-medium";

fn headers() -> ImageHeaders {
    BTreeMap::from([("x-request-id".to_owned(), "req-1".to_owned())])
}

fn reply(status: u16, body: &str) -> ImageHttpResponse {
    ImageHttpResponse {
        status_code: status,
        headers: headers(),
        body: body.as_bytes().to_vec(),
    }
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn size(width: i64, height: i64) -> Json {
    Json::Object(IndexMap::from([
        ("width".to_owned(), number(width)),
        ("height".to_owned(), number(height)),
    ]))
}

fn params(entries: &[(&str, Json)]) -> ImageParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn hidden(entries: &[(&str, Json)]) -> HiddenParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn image(b64_json: Option<&str>, url: Option<&str>, revised_prompt: Option<&str>) -> ImageData {
    ImageData {
        b64_json: b64_json.map(str::to_owned),
        revised_prompt: revised_prompt.map(str::to_owned),
        url: url.map(str::to_owned),
    }
}

fn url_image(url: &str) -> ImageData {
    image(None, Some(url), None)
}

fn response(data: Option<Vec<ImageData>>) -> ImageResponse {
    ImageResponse {
        created: 1,
        data,
        usage: None,
    }
}

fn transformed(body: &str) -> (ImageResponse, HiddenParams) {
    let mut response = response(Some(Vec::new()));
    let mut hidden_params = hidden(&[]);
    transform_reply(&mut response, &mut hidden_params, &reply(200, body)).expect("a reply");
    (response, hidden_params)
}

#[rstest]
#[case::bare(
    "stable-diffusion-v35-medium",
    "https://fal.run/fal-ai/stable-diffusion-v35-medium"
)]
#[case::prefixed(
    "fal-ai/stable-diffusion-v35-medium",
    "https://fal.run/fal-ai/stable-diffusion-v35-medium"
)]
#[case::custom(
    "custom/stable-diffusion-v35-medium",
    "https://fal.run/custom/stable-diffusion-v35-medium"
)]
#[case::nested("a/b/c", "https://fal.run/a/b/c")]
#[case::slash_first("/x", "https://fal.run//x")]
#[case::empty("", "https://fal.run/fal-ai/")]
#[case::prefix_only("fal-ai/", "https://fal.run/fal-ai/")]
fn stable_diffusion_url_keeps_or_adds_the_fal_prefix(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(
        complete_url(&FalImageSettings::default(), None, model),
        expected
    );
    assert_eq!(DEFAULT_BASE_URL, "https://fal.run");
}

#[test]
fn stable_diffusion_url_trims_the_base_and_prefers_an_explicit_one() {
    let settings = FalImageSettings::default();
    assert_eq!(
        complete_url(&settings, Some("https://proxy.test/fal///"), "fal-ai/sd"),
        "https://proxy.test/fal/fal-ai/sd"
    );
    assert_eq!(
        complete_url(&settings, Some(""), MODEL),
        format!("{DEFAULT_BASE_URL}/fal-ai/{MODEL}")
    );
    let environment = FalImageSettings {
        api_key: None,
        api_base: "https://environment.test/fal/".to_owned(),
    };
    assert_eq!(
        complete_url(&environment, None, MODEL),
        format!("https://environment.test/fal/fal-ai/{MODEL}")
    );
}

#[rstest]
#[case::square_hd("1024x1024", "square_hd")]
#[case::square("512x512", "square")]
#[case::portrait("768x1024", "portrait_4_3")]
#[case::portrait_16_9("576x1024", "portrait_16_9")]
#[case::landscape("1024x768", "landscape_4_3")]
#[case::landscape_16_9("1024x576", "landscape_16_9")]
fn stable_diffusion_size_aliases_are_presets(#[case] requested: &str, #[case] preset: &str) {
    assert_eq!(
        map_image_size(&text(requested)).expect("a preset"),
        text(preset)
    );
}

#[rstest]
#[case::ideogram_presets("1536x1024", size(1536, 1024))]
#[case::ideogram_tall("1024x1536", size(1024, 1536))]
#[case::padded(" 1024x1024 ", size(1024, 1024))]
#[case::padded_preset(" 512x512 ", size(512, 512))]
#[case::spaced("800 x 600", size(800, 600))]
#[case::plus("+1024x768", size(1024, 768))]
#[case::minus("-1024x768", size(-1024, 768))]
#[case::underscored("1_024x768", size(1024, 768))]
#[case::zeros("0x0", size(0, 0))]
fn stable_diffusion_size_parses_two_integer_parts(#[case] requested: &str, #[case] expected: Json) {
    assert_eq!(map_image_size(&text(requested)).expect("a size"), expected);
}

#[rstest]
#[case::empty("")]
#[case::blank("   ")]
#[case::auto("auto")]
#[case::width_only("1024")]
#[case::missing_height("1024x")]
#[case::three_parts("1x2x3")]
#[case::upper_x("1536X1024")]
#[case::fal_separator("1024-x-768")]
#[case::fraction("1.5x2")]
#[case::words("abcxdef")]
#[case::two_underscores("1__0x20")]
#[case::past_i64("99999999999999999999x1")]
#[case::min_i64("-9223372036854775808x1")]
#[case::arabic_digits("\u{0661}\u{0662}\u{0663}x456")]
fn stable_diffusion_size_falls_back_for_malformed_text(#[case] requested: &str) {
    assert_eq!(
        map_image_size(&text(requested)).expect("a fallback"),
        text(FALLBACK_SIZE)
    );
}

#[rstest]
#[case::number(number(7))]
#[case::null(Json::Null)]
#[case::truth(Json::Bool(true))]
#[case::list(Json::List(vec![number(1)]))]
#[case::object(size(10, 20))]
fn stable_diffusion_size_rejects_other_values(#[case] given: Json) {
    match map_image_size(&given) {
        Err(StableDiffusionError::SizeNotAString) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn stable_diffusion_mapping_translates_n_size_and_response_format() {
    let non_default = params(&[
        ("n", number(2)),
        ("size", text("1536x1024")),
        ("response_format", text("url")),
    ]);
    let optional = params(&[("seed", number(9))]);
    let mapped = map_openai_params(&non_default, optional, MODEL, false).expect("params");
    assert_eq!(
        mapped,
        params(&[
            ("num_images", number(2)),
            ("image_size", size(1536, 1024)),
            ("output_format", text("jpeg")),
            ("seed", number(9)),
        ])
    );
}

#[rstest]
#[case::url(text("url"), text("jpeg"))]
#[case::base64(text("b64_json"), text("jpeg"))]
#[case::png(text("png"), text("png"))]
#[case::number(number(5), number(5))]
#[case::null(Json::Null, Json::Null)]
fn stable_diffusion_mapping_turns_only_the_openai_formats_into_jpeg(
    #[case] requested: Json,
    #[case] expected: Json,
) {
    let non_default = params(&[("response_format", requested)]);
    let mapped = map_openai_params(&non_default, ImageParams::new(), MODEL, false)
        .expect("a response format");
    assert_eq!(mapped.get("output_format"), Some(&expected));
}

#[test]
fn stable_diffusion_mapping_checks_the_original_openai_name() {
    let non_default = params(&[("n", number(2)), ("size", text("1024x1024"))]);
    let optional = params(&[("num_images", number(9)), ("size", text("512x512"))]);
    let mapped = map_openai_params(&non_default, optional, MODEL, false).expect("an existing size");
    assert_eq!(
        mapped,
        params(&[("num_images", number(2)), ("size", text("512x512"))])
    );
}

#[test]
fn stable_diffusion_mapping_overwrites_the_destination_not_named_by_the_input() {
    let non_default = params(&[("n", number(2)), ("response_format", text("b64_json"))]);
    let optional = params(&[("num_images", number(9)), ("output_format", text("png"))]);
    let mapped =
        map_openai_params(&non_default, optional, MODEL, false).expect("overwritten destinations");
    assert_eq!(
        mapped,
        params(&[("num_images", number(2)), ("output_format", text("jpeg")),])
    );
}

#[test]
fn stable_diffusion_mapping_overwrites_the_image_size_destination() {
    let non_default = params(&[("size", text("1024x768"))]);
    let optional = params(&[("image_size", text("square"))]);
    let mapped =
        map_openai_params(&non_default, optional, MODEL, false).expect("an overwritten image size");
    assert_eq!(mapped, params(&[("image_size", size(1024, 768))]));
}

#[test]
fn stable_diffusion_mapping_rejects_unsupported_parameters() {
    let non_default = params(&[("seed", number(1)), ("user", text("alice"))]);
    match map_openai_params(&non_default, ImageParams::new(), MODEL, false) {
        Err(StableDiffusionError::UnsupportedParameters { parameters, model }) => {
            assert_eq!(parameters, ["seed", "user"]);
            assert_eq!(model, MODEL);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn stable_diffusion_mapping_drops_unsupported_parameters() {
    let non_default = params(&[("seed", number(1)), ("n", number(2))]);
    let mapped =
        map_openai_params(&non_default, ImageParams::new(), MODEL, true).expect("dropped params");
    assert_eq!(mapped, params(&[("num_images", number(2))]));
}

#[test]
fn stable_diffusion_mapping_keeps_an_unsupported_parameter_already_supplied() {
    let non_default = params(&[("seed", number(1))]);
    let optional = params(&[("seed", number(9))]);
    let mapped =
        map_openai_params(&non_default, optional, MODEL, false).expect("an existing parameter");
    assert_eq!(mapped.get("seed"), Some(&number(9)));
}

#[test]
fn stable_diffusion_mapping_reports_a_nonstring_size() {
    let non_default = params(&[("size", size(10, 20))]);
    match map_openai_params(&non_default, ImageParams::new(), MODEL, false) {
        Err(StableDiffusionError::SizeNotAString) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn stable_diffusion_request_body_overlays_options_after_prompt() {
    let body = image_generation_body(
        "a prompt",
        params(&[
            ("num_images", number(2)),
            ("image_size", text("landscape_4_3")),
        ]),
    );
    assert_eq!(
        body.keys().cloned().collect::<Vec<_>>(),
        ["prompt", "num_images", "image_size"]
    );
    let overlaid = image_generation_body("a prompt", params(&[("prompt", text("later"))]));
    assert_eq!(overlaid.get("prompt"), Some(&text("later")));
    assert_eq!(overlaid.keys().cloned().collect::<Vec<_>>(), ["prompt"]);
}

#[test]
fn stable_diffusion_reply_appends_url_only_images() {
    let mut response = response(Some(vec![image(Some("aGk="), None, Some("revised"))]));
    let mut hidden_params = hidden(&[("existing", text("keep"))]);
    transform_reply(
        &mut response,
        &mut hidden_params,
        &reply(
            200,
            r#"{"images": ["https://a.test/1.png", {"url": "https://b.test/2.png", "b64_json": "aGk=", "width": 10, "height": 20, "content_type": "image/png"}]}"#,
        ),
    )
    .expect("a reply");
    assert_eq!(
        response.data,
        Some(vec![
            image(Some("aGk="), None, Some("revised")),
            url_image("https://a.test/1.png"),
            url_image("https://b.test/2.png"),
        ])
    );
    assert_eq!(hidden_params, hidden(&[("existing", text("keep"))]));
}

#[test]
fn stable_diffusion_reply_creates_data_for_a_response_without_images() {
    let (appended, _) = transformed(r#"{"images": ["https://a.test/1.png"]}"#);
    assert_eq!(appended.data, Some(vec![url_image("https://a.test/1.png")]));
    let mut empty = response(None);
    transform_reply(&mut empty, &mut hidden(&[]), &reply(200, "{}")).expect("an empty reply");
    assert_eq!(empty.data, Some(Vec::new()));
}

#[rstest]
#[case::object(r#"{"images": {"url": "https://a.test/1.png"}}"#)]
#[case::text(r#"{"images": "https://a.test/1.png"}"#)]
#[case::number(r#"{"images": 1}"#)]
#[case::null(r#"{"images": null}"#)]
#[case::missing("{}")]
fn stable_diffusion_reply_requires_an_image_list(#[case] body: &str) {
    let (appended, _) = transformed(body);
    assert_eq!(appended.data, Some(Vec::new()));
}

#[rstest]
#[case::number(r#"{"images": [7]}"#)]
#[case::truth(r#"{"images": [true]}"#)]
#[case::null(r#"{"images": [null]}"#)]
#[case::list(r#"{"images": [[1]]}"#)]
fn stable_diffusion_reply_ignores_members_that_are_not_strings_or_maps(#[case] body: &str) {
    let (appended, _) = transformed(body);
    assert_eq!(appended.data, Some(Vec::new()));
}

#[rstest]
#[case::empty_object(r#"{"images": [{}]}"#)]
#[case::null_url(r#"{"images": [{"url": null}]}"#)]
fn stable_diffusion_reply_keeps_a_missing_url_as_a_null_url(#[case] body: &str) {
    let (appended, _) = transformed(body);
    assert_eq!(appended.data, Some(vec![image(None, None, None)]));
}

#[rstest]
#[case::number_url(r#"{"images": [{"url": 5}]}"#, 0)]
#[case::truth_url(r#"{"images": [{"url": false}]}"#, 0)]
#[case::second_member(r#"{"images": ["https://a.test/1.png", {"url": [1]}]}"#, 1)]
fn stable_diffusion_reply_rejects_a_map_with_an_invalid_url(
    #[case] body: &str,
    #[case] index: usize,
) {
    let mut response = response(Some(vec![url_image("https://existing.test/0.png")]));
    let mut hidden_params = hidden(&[]);
    match transform_reply(&mut response, &mut hidden_params, &reply(200, body)) {
        Err(StableDiffusionError::InvalidImageUrl { index: rejected }) => {
            assert_eq!(rejected, index);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        response.data,
        Some(vec![url_image("https://existing.test/0.png")])
    );
}

#[test]
fn stable_diffusion_reply_copies_every_hidden_field_by_presence() {
    let (_, hidden_params) = transformed(
        r#"{"seed": null, "timings": {"inference": 2.5}, "has_nsfw_concepts": [false], "prompt": "ignored"}"#,
    );
    assert_eq!(
        hidden_params,
        hidden(&[
            ("seed", Json::Null),
            (
                "timings",
                Json::Object(IndexMap::from([(
                    "inference".to_owned(),
                    serde_json::from_str::<Json>("2.5").expect("a float")
                )]))
            ),
            ("has_nsfw_concepts", Json::List(vec![Json::Bool(false)])),
        ])
    );
}

#[test]
fn stable_diffusion_reply_copies_only_the_fields_the_reply_carries() {
    let (_, hidden_params) = transformed(r#"{"seed": 42}"#);
    assert_eq!(hidden_params, hidden(&[("seed", number(42))]));
    let (_, none) = transformed(r#"{"timings": {"inference": 1}}"#);
    assert_eq!(
        none,
        hidden(&[(
            "timings",
            Json::Object(IndexMap::from([("inference".to_owned(), number(1))]))
        )])
    );
}

#[test]
fn stable_diffusion_reply_replaces_an_existing_hidden_value() {
    let mut response = response(Some(Vec::new()));
    let mut hidden_params = hidden(&[("seed", number(7)), ("existing", text("keep"))]);
    transform_reply(
        &mut response,
        &mut hidden_params,
        &reply(200, r#"{"seed": 42}"#),
    )
    .expect("a reply");
    assert_eq!(
        hidden_params,
        hidden(&[("seed", number(42)), ("existing", text("keep"))])
    );
}

#[test]
fn stable_diffusion_reply_does_not_copy_hidden_fields_when_a_member_fails() {
    let mut response = response(Some(Vec::new()));
    let mut hidden_params = hidden(&[("seed", number(7))]);
    let failed = transform_reply(
        &mut response,
        &mut hidden_params,
        &reply(
            200,
            r#"{"images": [{"url": 5}], "seed": 42, "timings": {}}"#,
        ),
    );
    assert!(matches!(
        failed,
        Err(StableDiffusionError::InvalidImageUrl { index: 0 })
    ));
    assert_eq!(hidden_params, hidden(&[("seed", number(7))]));
}

#[test]
fn stable_diffusion_reply_keeps_the_reply_status_and_headers_on_a_decode_failure() {
    let mut response = response(Some(Vec::new()));
    let mut hidden_params = hidden(&[]);
    match transform_reply(&mut response, &mut hidden_params, &reply(502, "{")) {
        Err(StableDiffusionError::Fal(FalImageError::ReplyDecode {
            status,
            headers: carried,
            ..
        })) => {
            assert_eq!(status, 502);
            assert_eq!(carried, headers());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(response.data, Some(Vec::new()));
}

#[test]
fn stable_diffusion_reply_rejects_a_non_object_root() {
    let mut response = response(Some(Vec::new()));
    let mut hidden_params = hidden(&[]);
    match transform_reply(&mut response, &mut hidden_params, &reply(201, "[]")) {
        Err(error @ StableDiffusionError::Fal(FalImageError::ReplyDecode { .. })) => {
            assert_eq!(error.status(), Some(201));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn stable_diffusion_reply_rejects_an_oversized_reply() {
    let mut response = response(Some(Vec::new()));
    let mut hidden_params = hidden(&[]);
    let oversized = ImageHttpResponse {
        status_code: 200,
        headers: headers(),
        body: vec![b' '; MAX_FAL_REPLY_BYTES + 1],
    };
    match transform_reply(&mut response, &mut hidden_params, &oversized) {
        Err(error @ StableDiffusionError::Fal(FalImageError::ReplyTooLarge { .. })) => {
            assert_eq!(error.status(), Some(200));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn stable_diffusion_reply_accepts_a_reply_at_the_bound() {
    let mut at_bound = vec![b' '; MAX_FAL_REPLY_BYTES];
    at_bound[..14].copy_from_slice(br#"{"images": []}"#);
    let mut response = response(None);
    let mut hidden_params = hidden(&[]);
    transform_reply(
        &mut response,
        &mut hidden_params,
        &ImageHttpResponse {
            status_code: 200,
            headers: headers(),
            body: at_bound,
        },
    )
    .expect("a reply at the bound");
    assert_eq!(response.data, Some(Vec::new()));
}

#[test]
fn stable_diffusion_errors_carry_a_status_only_for_decoder_failures() {
    assert_eq!(StableDiffusionError::SizeNotAString.status(), None);
    assert_eq!(
        StableDiffusionError::InvalidImageUrl { index: 0 }.status(),
        None
    );
    assert_eq!(
        StableDiffusionError::UnsupportedParameters {
            parameters: vec!["seed".to_owned()],
            model: MODEL.to_owned(),
        }
        .status(),
        None
    );
    assert_eq!(
        StableDiffusionError::Fal(FalImageError::MissingApiKey).status(),
        None
    );
}
