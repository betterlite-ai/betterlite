#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use indexmap::IndexMap;
use rstest::rstest;

use super::GENERATION_ENDPOINT;
use super::error::IdeogramError;
use super::params::map_openai_params;
use super::reply::transform_reply;
use super::size::{FALLBACK_SIZE, map_image_size};
use crate::api::fal::images::decode::MAX_FAL_REPLY_BYTES;
use crate::api::fal::images::error::FalImageError;
use crate::api::fal::images::params::image_generation_body;
use crate::api::fal::images::settings::{DEFAULT_BASE_URL, FalImageSettings};

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

#[test]
fn ideogram_endpoint_is_fixed() {
    assert_eq!(GENERATION_ENDPOINT, "fal-ai/ideogram/v3");
    let settings = FalImageSettings::default();
    assert_eq!(
        settings.complete_url(None, GENERATION_ENDPOINT),
        format!("{DEFAULT_BASE_URL}/fal-ai/ideogram/v3")
    );
    assert_eq!(
        settings.complete_url(Some("https://proxy.test/fal///"), GENERATION_ENDPOINT),
        "https://proxy.test/fal/fal-ai/ideogram/v3"
    );
    let environment = FalImageSettings {
        api_key: None,
        api_base: "https://environment.test".to_owned(),
    };
    assert_eq!(
        environment.complete_url(None, GENERATION_ENDPOINT),
        "https://environment.test/fal-ai/ideogram/v3"
    );
}

#[rstest]
#[case::square_hd("1024x1024", "square_hd")]
#[case::square("512x512", "square")]
#[case::landscape("1024x768", "landscape_4_3")]
#[case::portrait("768x1024", "portrait_4_3")]
#[case::wide("1536x1024", "landscape_16_9")]
#[case::tall("1024x1536", "portrait_16_9")]
fn ideogram_size_aliases_are_presets(#[case] requested: &str, #[case] preset: &str) {
    assert_eq!(map_image_size(&text(requested)), text(preset));
}

#[rstest]
#[case::spaces(" 1024x1024 ", "square_hd")]
#[case::tabs("\t768x1024\n", "portrait_4_3")]
#[case::no_break_space("\u{a0}1536x1024\u{a0}", "landscape_16_9")]
#[case::padded_dimensions(" 576x1024 ", size(576, 1024))]
fn ideogram_size_trims_before_mapping(#[case] requested: &str, #[case] expected: Json) {
    assert_eq!(map_image_size(&text(requested)), expected);
}

#[rstest]
#[case::spaced("800 x 600", size(800, 600))]
#[case::plus("+1024x768", size(1024, 768))]
#[case::minus("-1024x768", size(-1024, 768))]
#[case::underscored("1_024x768", size(1024, 768))]
#[case::zeros("0x0", size(0, 0))]
#[case::trimmed_parts("1024x 768", size(1024, 768))]
fn ideogram_size_parses_two_integer_parts(#[case] requested: &str, #[case] expected: Json) {
    assert_eq!(map_image_size(&text(requested)), expected);
}

#[rstest]
#[case::empty("")]
#[case::blank("   ")]
#[case::auto("auto")]
#[case::upper_auto("AUTO")]
#[case::width_only("1024")]
#[case::missing_height("1024x")]
#[case::missing_width("x1024")]
#[case::three_parts("1024x768x")]
#[case::two_separators("1x2x3")]
#[case::upper_x("1536X1024")]
#[case::fal_separator("1024-x-768")]
#[case::fraction("1.5x2")]
#[case::words("abcxdef")]
#[case::two_underscores("1__0x20")]
#[case::past_i64("99999999999999999999x1")]
#[case::min_i64("-9223372036854775808x1")]
#[case::arabic_digits("\u{0661}\u{0662}\u{0663}x456")]
#[case::information_separators("\u{1c}512x512\u{1c}")]
fn ideogram_size_falls_back_for_malformed_text(#[case] requested: &str) {
    assert_eq!(map_image_size(&text(requested)), text(FALLBACK_SIZE));
}

#[rstest]
#[case::extra_fields(
    Json::Object(IndexMap::from([
        ("width".to_owned(), number(10)),
        ("height".to_owned(), number(20)),
        ("preset".to_owned(), text("square")),
    ])),
    size(10, 20)
)]
#[case::height_first(
    Json::Object(IndexMap::from([
        ("height".to_owned(), number(20)),
        ("width".to_owned(), number(10)),
    ])),
    size(10, 20)
)]
#[case::bools(
    Json::Object(IndexMap::from([
        ("width".to_owned(), Json::Bool(true)),
        ("height".to_owned(), Json::Bool(false)),
        ("preset".to_owned(), text("square")),
    ])),
    Json::Object(IndexMap::from([
        ("width".to_owned(), Json::Bool(true)),
        ("height".to_owned(), Json::Bool(false)),
    ]))
)]
#[case::past_i64(
    Json::Object(IndexMap::from([
        ("width".to_owned(), Json::Number(serde_json::Number::from(u64::MAX))),
        ("height".to_owned(), number(2)),
        ("preset".to_owned(), text("square")),
    ])),
    Json::Object(IndexMap::from([
        ("width".to_owned(), Json::Number(serde_json::Number::from(u64::MAX))),
        ("height".to_owned(), number(2)),
    ]))
)]
fn ideogram_size_keeps_only_two_integer_dimensions(#[case] given: Json, #[case] expected: Json) {
    assert_eq!(map_image_size(&given), expected);
}

#[rstest]
#[case::float_height(
    Json::Object(IndexMap::from([
        ("width".to_owned(), number(10)),
        ("height".to_owned(), serde_json::from_str::<Json>("1.5").expect("a float")),
        ("preset".to_owned(), text("square")),
    ]))
)]
#[case::float_literal(
    Json::Object(IndexMap::from([
        ("width".to_owned(), serde_json::from_str::<Json>("1e10").expect("a float")),
        ("height".to_owned(), number(2)),
    ]))
)]
#[case::text_height(
    Json::Object(IndexMap::from([
        ("width".to_owned(), number(10)),
        ("height".to_owned(), text("20")),
    ]))
)]
#[case::missing_height(Json::Object(IndexMap::from([("width".to_owned(), number(10))])))]
#[case::empty(Json::Object(IndexMap::new()))]
fn ideogram_size_passes_an_incomplete_object_through(#[case] given: Json) {
    assert_eq!(map_image_size(&given), given);
}

#[rstest]
#[case::number(number(7))]
#[case::null(Json::Null)]
#[case::truth(Json::Bool(true))]
#[case::list(Json::List(vec![number(1), number(2)]))]
fn ideogram_size_passes_other_values_through(#[case] given: Json) {
    assert_eq!(map_image_size(&given), given);
}

#[test]
fn ideogram_mapping_translates_n_and_size() {
    let non_default = params(&[
        ("n", number(2)),
        ("size", text("1536x1024")),
        ("response_format", text("url")),
    ]);
    let optional = params(&[("seed", number(9))]);
    let mapped =
        map_openai_params(&non_default, optional, "fal-ai/ideogram/v3", false).expect("params");
    assert_eq!(
        mapped,
        params(&[
            ("num_images", number(2)),
            ("image_size", text("landscape_16_9")),
            ("seed", number(9)),
        ])
    );
}

#[test]
fn ideogram_mapping_checks_the_original_openai_name() {
    let non_default = params(&[("n", number(2)), ("size", text("1024x1024"))]);
    let optional = params(&[("num_images", number(9)), ("size", text("512x512"))]);
    let mapped = map_openai_params(&non_default, optional, "fal-ai/ideogram/v3", false)
        .expect("an existing size");
    assert_eq!(
        mapped,
        params(&[("num_images", number(2)), ("size", text("512x512"))])
    );
}

#[test]
fn ideogram_mapping_overwrites_a_destination_the_input_does_not_name() {
    let non_default = params(&[("size", text("1024x768"))]);
    let optional = params(&[("image_size", text("square"))]);
    let mapped = map_openai_params(&non_default, optional, "fal-ai/ideogram/v3", false)
        .expect("an overwritten destination");
    assert_eq!(mapped, params(&[("image_size", size(1024, 768))]));
}

#[test]
fn ideogram_mapping_treats_a_destination_added_earlier_as_present() {
    let non_default = params(&[("size", text("1024x768")), ("image_size", size(1, 2))]);
    let mapped = map_openai_params(
        &non_default,
        ImageParams::new(),
        "fal-ai/ideogram/v3",
        false,
    )
    .expect("a destination the earlier key supplied");
    assert_eq!(mapped, params(&[("image_size", size(1024, 768))]));
}

#[test]
fn ideogram_mapping_rejects_unsupported_parameters() {
    let non_default = params(&[("seed", number(1)), ("user", text("alice"))]);
    match map_openai_params(
        &non_default,
        ImageParams::new(),
        "fal-ai/ideogram/v3",
        false,
    ) {
        Err(IdeogramError::UnsupportedParameters { parameters, model }) => {
            assert_eq!(parameters, ["seed"]);
            assert_eq!(model, "fal-ai/ideogram/v3");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn ideogram_mapping_drops_unsupported_parameters() {
    let non_default = params(&[("seed", number(1)), ("n", number(2))]);
    let mapped = map_openai_params(&non_default, ImageParams::new(), "fal-ai/ideogram/v3", true)
        .expect("dropped params");
    assert_eq!(mapped, params(&[("num_images", number(2))]));
}

#[test]
fn ideogram_mapping_keeps_an_unsupported_parameter_already_supplied() {
    let non_default = params(&[("seed", number(1))]);
    let optional = params(&[("seed", number(9))]);
    let mapped = map_openai_params(&non_default, optional, "fal-ai/ideogram/v3", false)
        .expect("an existing parameter");
    assert_eq!(mapped.get("seed"), Some(&number(9)));
}

#[test]
fn ideogram_request_body_overlays_options_after_prompt() {
    let body = image_generation_body(
        "a prompt",
        params(&[("num_images", number(2)), ("image_size", text("square_hd"))]),
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
fn ideogram_reply_appends_url_only_images() {
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
fn ideogram_reply_creates_data_for_a_response_without_images() {
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
fn ideogram_reply_requires_an_image_list(#[case] body: &str) {
    let (appended, _) = transformed(body);
    assert_eq!(appended.data, Some(Vec::new()));
}

#[rstest]
#[case::empty_object(r#"{"images": [{}]}"#)]
#[case::null_entry(r#"{"images": [null]}"#)]
#[case::null_url(r#"{"images": [{"url": null}]}"#)]
#[case::text_entry(r#"{"images": ["https://a.test/1.png"]}"#)]
fn ideogram_reply_keeps_a_missing_url_as_a_null_url(#[case] body: &str) {
    let (appended, _) = transformed(body);
    assert_eq!(appended.data, Some(vec![image(None, None, None)]));
}

#[rstest]
#[case::number(r#"{"images": [7]}"#, 0)]
#[case::truth(r#"{"images": [true]}"#, 0)]
#[case::list(r#"{"images": [[1]]}"#, 0)]
#[case::number_url(r#"{"images": [{"url": 5}]}"#, 0)]
#[case::object_url(r#"{"images": [{"url": {"a": 1}}]}"#, 0)]
#[case::second_member(r#"{"images": ["https://a.test/1.png", 7]}"#, 1)]
fn ideogram_reply_rejects_a_member_that_is_not_a_url(#[case] body: &str, #[case] index: usize) {
    let mut response = response(Some(vec![url_image("https://existing.test/0.png")]));
    let mut hidden_params = hidden(&[]);
    match transform_reply(&mut response, &mut hidden_params, &reply(200, body)) {
        Err(IdeogramError::InvalidImageUrl { index: rejected }) => assert_eq!(rejected, index),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        response.data,
        Some(vec![url_image("https://existing.test/0.png")])
    );
}

#[rstest]
#[case::number(r#"{"seed": 42}"#, number(42))]
#[case::null(r#"{"seed": null}"#, Json::Null)]
#[case::object(r#"{"seed": {"value": 1}}"#, Json::Object(IndexMap::from([("value".to_owned(), number(1))])))]
fn ideogram_reply_copies_a_present_seed(#[case] body: &str, #[case] seed: Json) {
    let mut response = response(Some(Vec::new()));
    let mut hidden_params = hidden(&[("seed", number(7)), ("existing", text("keep"))]);
    transform_reply(&mut response, &mut hidden_params, &reply(200, body)).expect("a reply");
    assert_eq!(
        hidden_params,
        hidden(&[("seed", seed), ("existing", text("keep"))])
    );
}

#[test]
fn ideogram_reply_ignores_other_hidden_fields() {
    let (_, hidden_params) = transformed(
        r#"{"timings": {"inference": 2}, "has_nsfw_concepts": [false], "prompt": "ignored"}"#,
    );
    assert!(hidden_params.is_empty());
}

#[test]
fn ideogram_reply_does_not_copy_a_seed_when_a_member_fails() {
    let mut response = response(Some(Vec::new()));
    let mut hidden_params = hidden(&[("seed", number(7))]);
    let failed = transform_reply(
        &mut response,
        &mut hidden_params,
        &reply(200, r#"{"images": [7], "seed": 42}"#),
    );
    assert!(matches!(
        failed,
        Err(IdeogramError::InvalidImageUrl { index: 0 })
    ));
    assert_eq!(hidden_params, hidden(&[("seed", number(7))]));
}

#[test]
fn ideogram_reply_keeps_the_reply_status_and_headers_on_a_decode_failure() {
    let mut response = response(Some(Vec::new()));
    let mut hidden_params = hidden(&[]);
    match transform_reply(&mut response, &mut hidden_params, &reply(502, "{")) {
        Err(error @ IdeogramError::Fal(FalImageError::ReplyDecode { .. })) => {
            assert_eq!(error.status(), Some(502));
        }
        other => panic!("{other:?}"),
    }
    match transform_reply(&mut response, &mut hidden_params, &reply(502, "{")) {
        Err(IdeogramError::Fal(FalImageError::ReplyDecode {
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
fn ideogram_reply_rejects_a_non_object_root() {
    let mut response = response(Some(Vec::new()));
    let mut hidden_params = hidden(&[]);
    match transform_reply(&mut response, &mut hidden_params, &reply(201, "[]")) {
        Err(error @ IdeogramError::Fal(FalImageError::ReplyNotAnObject { .. })) => {
            assert_eq!(error.status(), Some(201));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn ideogram_reply_rejects_an_oversized_reply() {
    let mut response = response(Some(Vec::new()));
    let mut hidden_params = hidden(&[]);
    let oversized = ImageHttpResponse {
        status_code: 200,
        headers: headers(),
        body: vec![b' '; MAX_FAL_REPLY_BYTES + 1],
    };
    match transform_reply(&mut response, &mut hidden_params, &oversized) {
        Err(IdeogramError::Fal(FalImageError::ReplyTooLarge {
            status,
            headers: carried,
            limit,
        })) => {
            assert_eq!(status, 200);
            assert_eq!(carried, headers());
            assert_eq!(limit, MAX_FAL_REPLY_BYTES);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn ideogram_reply_accepts_a_reply_at_the_bound() {
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
fn ideogram_errors_carry_a_status_only_for_decoder_failures() {
    assert_eq!(IdeogramError::InvalidImageUrl { index: 0 }.status(), None);
    assert_eq!(
        IdeogramError::UnsupportedParameters {
            parameters: vec!["seed".to_owned()],
            model: "fal-ai/ideogram/v3".to_owned(),
        }
        .status(),
        None
    );
    assert_eq!(
        IdeogramError::Fal(FalImageError::MissingApiKey).status(),
        None
    );
}
