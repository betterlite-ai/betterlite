#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageParams, ImageResponse,
};
use indexmap::IndexMap;
use rstest::rstest;

use super::ENDPOINT;
use super::append_singular_image;
use super::map_openai_params;
use super::size::map_aspect_ratio;
use crate::api::fal::images::UrlOnlyImageError;
use crate::api::fal::images::decode::MAX_FAL_REPLY_BYTES;
use crate::api::fal::images::error::FalImageError;

fn headers() -> ImageHeaders {
    BTreeMap::from([("x-request-id".to_owned(), "req-1".to_owned())])
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn params(entries: &[(&str, Json)]) -> ImageParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn size_object() -> Json {
    Json::Object(IndexMap::from([("width".to_owned(), number(5))]))
}

fn url_image(url: &str) -> ImageData {
    ImageData {
        b64_json: None,
        revised_prompt: None,
        url: Some(url.to_owned()),
    }
}

fn appended(body: &str) -> ImageResponse {
    let mut response = ImageResponse::default();
    append_singular_image(&mut response, 200, &headers(), body.as_bytes()).expect("a reply");
    response
}

#[rstest]
#[case::square("1024x1024", "1:1")]
#[case::small_square("512x512", "1:1")]
#[case::wide("1792x1024", "16:9")]
#[case::tall("1024x1792", "9:16")]
#[case::landscape("1024x768", "4:3")]
#[case::portrait("768x1024", "3:4")]
#[case::native_landscape("1280x960", "4:3")]
#[case::native_portrait("960x1280", "3:4")]
fn bria_exact_aliases_map_to_their_ratios(#[case] size: &str, #[case] ratio: &str) {
    assert_eq!(map_aspect_ratio(size), ratio);
}

#[rstest]
#[case::square_center("1000x1000", "1:1")]
#[case::square_floor("950x1000", "1:1")]
#[case::square_ceiling("1050x1000", "1:1")]
#[case::square_gap_low("949x1000", "1:1")]
#[case::wide_floor("1700x1000", "16:9")]
#[case::wide_gap("1699x1000", "1:1")]
#[case::tall_ceiling("600x1000", "9:16")]
#[case::tall_gap("601x1000", "1:1")]
#[case::four_thirds_overlap("1300x1000", "4:3")]
#[case::four_thirds_ceiling("1400x1000", "4:3")]
#[case::four_thirds_gap("1401x1000", "1:1")]
#[case::five_fourths("1250x1000", "5:4")]
#[case::five_fourths_floor("1200x1000", "5:4")]
#[case::five_fourths_gap("1199x1000", "1:1")]
#[case::three_fourths_overlap("700x1000", "3:4")]
#[case::three_fourths_ceiling("800x1000", "3:4")]
#[case::two_thirds("699x1000", "2:3")]
#[case::two_thirds_floor("650x1000", "2:3")]
#[case::two_thirds_gap("649x1000", "1:1")]
#[case::three_halves_floor("1450x1000", "3:2")]
#[case::three_halves_ceiling("1550x1000", "3:2")]
#[case::three_halves_gap_low("1449x1000", "1:1")]
#[case::three_halves_gap_high("1551x1000", "1:1")]
#[case::four_fifths("850x1000", "4:5")]
#[case::four_fifths_gap("851x1000", "1:1")]
#[case::four_fifths_shadowed("750x1000", "3:4")]
fn bria_ratio_intervals_keep_the_pinned_order(#[case] size: &str, #[case] ratio: &str) {
    assert_eq!(map_aspect_ratio(size), ratio);
}

#[rstest]
#[case::zero_width("0x5", "9:16")]
#[case::zero_height("5x0", "1:1")]
#[case::zero_both("0x0", "1:1")]
#[case::negative_width("-10x20", "9:16")]
#[case::negative_height("10x-20", "9:16")]
#[case::negative_both("-10x-20", "9:16")]
#[case::upper_x("1024X1024", "1:1")]
#[case::fraction("1.5x2", "1:1")]
#[case::spaced(" 1024x768", "4:3")]
#[case::trailing_space("1024x 768", "4:3")]
#[case::underscored("1_0x20", "9:16")]
#[case::fal_separator("10-x-20", "1:1")]
#[case::three_parts("1x2x3", "1:1")]
#[case::words("axb", "1:1")]
#[case::empty("", "1:1")]
#[case::auto("auto", "1:1")]
#[case::past_i64("999999999999999999999999x1", "1:1")]
#[case::min_i64("-9223372036854775808x20", "1:1")]
#[case::non_ascii_digits("١٠٢٤x٧٦٨", "1:1")]
fn bria_unreadable_sizes_become_square(#[case] size: &str, #[case] ratio: &str) {
    assert_eq!(map_aspect_ratio(size), ratio);
}

// a deviation: Python divides the exact integers, betterlite rounds each dimension to f64 first
#[test]
fn bria_wide_dimensions_round_before_dividing() {
    assert_eq!(
        map_aspect_ratio("15312238733059686x9007199254740993"),
        "16:9"
    );
}

#[test]
fn bria_mapping_ignores_n_and_response_format() {
    let non_default = params(&[
        ("n", number(3)),
        ("response_format", text("url")),
        ("size", text("1024x768")),
    ]);
    let mapped = map_openai_params(&non_default, ImageParams::new(), ENDPOINT, false)
        .expect("ignored options");
    assert_eq!(mapped, params(&[("aspect_ratio", text("4:3"))]));
}

#[test]
fn bria_mapping_keeps_an_existing_original_option() {
    let non_default = params(&[("size", text("1024x768"))]);
    let optional = params(&[("size", text("auto"))]);
    let mapped = map_openai_params(&non_default, optional, ENDPOINT, false)
        .expect("an existing original option");
    assert_eq!(mapped, params(&[("size", text("auto"))]));
}

#[test]
fn bria_mapping_overwrites_the_translated_destination() {
    let non_default = params(&[("size", text("1024x768"))]);
    let optional = params(&[
        ("aspect_ratio", text("3:2")),
        ("image_size", text("square")),
    ]);
    let mapped = map_openai_params(&non_default, optional, ENDPOINT, false)
        .expect("a translated destination");
    assert_eq!(
        mapped,
        params(&[
            ("aspect_ratio", text("4:3")),
            ("image_size", text("square")),
        ])
    );
}

#[test]
fn bria_mapping_rejects_the_first_unsupported_option() {
    let non_default = params(&[("seed", number(1)), ("user", text("alice"))]);
    match map_openai_params(&non_default, ImageParams::new(), ENDPOINT, false) {
        Err(UrlOnlyImageError::UnsupportedParameters { parameters, model }) => {
            assert_eq!(parameters, ["seed"]);
            assert_eq!(model, ENDPOINT);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn bria_mapping_drops_unsupported_options_with_drop_params() {
    let non_default = params(&[("seed", number(1)), ("n", number(2))]);
    let mapped = map_openai_params(&non_default, ImageParams::new(), ENDPOINT, true)
        .expect("dropped options");
    assert!(mapped.is_empty());
}

#[test]
fn bria_mapping_keeps_an_unsupported_option_already_present() {
    let non_default = params(&[("seed", number(1))]);
    let optional = params(&[("seed", number(9))]);
    let mapped =
        map_openai_params(&non_default, optional, ENDPOINT, false).expect("an existing option");
    assert_eq!(mapped.get("seed"), Some(&number(9)));
}

#[rstest]
#[case::number(number(5))]
#[case::null(Json::Null)]
#[case::truth(Json::Bool(true))]
#[case::object(size_object())]
fn bria_mapping_requires_a_text_size(#[case] size: Json) {
    let non_default = params(&[("size", size)]);
    match map_openai_params(&non_default, ImageParams::new(), ENDPOINT, false) {
        Err(UrlOnlyImageError::SizeNotText) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn bria_mapping_requires_a_text_size_with_drop_params() {
    let non_default = params(&[("size", number(5))]);
    match map_openai_params(&non_default, ImageParams::new(), ENDPOINT, true) {
        Err(UrlOnlyImageError::SizeNotText) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn bria_singular_image_appends_one_url_object() {
    let response = appended(
        r#"{"image": {"url": "https://a.test/1.png", "width": 10, "height": 20, "content_type": "image/png", "b64_json": "aGk=", "seed": 7}}"#,
    );
    assert_eq!(response.data, Some(vec![url_image("https://a.test/1.png")]));
}

#[rstest]
#[case::empty_object(r#"{"image": {}}"#)]
#[case::missing("{}")]
#[case::null(r#"{"image": null}"#)]
#[case::text(r#"{"image": "https://a.test/1.png"}"#)]
#[case::number(r#"{"image": 5}"#)]
#[case::empty_list(r#"{"image": []}"#)]
#[case::plural(r#"{"images": ["https://a.test/1.png"]}"#)]
fn bria_reply_without_a_nonempty_image_object_appends_nothing(#[case] body: &str) {
    assert_eq!(appended(body).data, Some(Vec::new()));
}

#[test]
fn bria_image_without_a_url_appends_a_null_url() {
    assert_eq!(
        appended(r#"{"image": {"url": null}}"#).data,
        Some(vec![url_image_null()])
    );
    assert_eq!(
        appended(r#"{"image": {"b64_json": "aGk="}}"#).data,
        Some(vec![url_image_null()])
    );
}

fn url_image_null() -> ImageData {
    ImageData {
        b64_json: None,
        revised_prompt: None,
        url: None,
    }
}

#[rstest]
#[case::number(r#"{"image": {"url": 5}}"#)]
#[case::truth(r#"{"image": {"url": true}}"#)]
#[case::list(r#"{"image": {"url": []}}"#)]
#[case::object(r#"{"image": {"url": {}}}"#)]
fn bria_image_url_must_be_text(#[case] body: &str) {
    let mut response = ImageResponse::default();
    match append_singular_image(&mut response, 200, &headers(), body.as_bytes()) {
        Err(UrlOnlyImageError::FieldNotText { field, model }) => {
            assert_eq!(field, "url");
            assert_eq!(model, ENDPOINT);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn bria_reply_keeps_existing_images() {
    let mut response = ImageResponse {
        created: 0,
        data: Some(vec![url_image("https://first.test/0.png")]),
        usage: None,
    };
    append_singular_image(
        &mut response,
        200,
        &headers(),
        br#"{"image": {"url": "https://a.test/1.png"}}"#,
    )
    .expect("a singular image");
    assert_eq!(
        response.data,
        Some(vec![
            url_image("https://first.test/0.png"),
            url_image("https://a.test/1.png"),
        ])
    );
}

#[test]
fn bria_malformed_reply_keeps_the_source_status_and_headers() {
    let mut response = ImageResponse::default();
    match append_singular_image(&mut response, 502, &headers(), b"{") {
        Err(UrlOnlyImageError::Reply(FalImageError::ReplyDecode {
            status,
            headers: carried,
            ..
        })) => {
            assert_eq!(status, 502);
            assert_eq!(carried, headers());
        }
        other => panic!("{other:?}"),
    }
}

#[rstest]
#[case::list(b"[]")]
#[case::text(br#""image""#)]
#[case::number(b"7")]
#[case::null(b"null")]
fn bria_reply_requires_an_object_root(#[case] body: &[u8]) {
    let mut response = ImageResponse::default();
    match append_singular_image(&mut response, 500, &headers(), body) {
        Err(UrlOnlyImageError::Reply(FalImageError::ReplyNotAnObject {
            status,
            headers: carried,
        })) => {
            assert_eq!(status, 500);
            assert_eq!(carried, headers());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn bria_reply_size_is_bounded() {
    let oversized = vec![b' '; MAX_FAL_REPLY_BYTES + 1];
    let mut response = ImageResponse::default();
    match append_singular_image(&mut response, 413, &headers(), &oversized) {
        Err(UrlOnlyImageError::Reply(FalImageError::ReplyTooLarge {
            status,
            headers: carried,
            limit,
        })) => {
            assert_eq!(status, 413);
            assert_eq!(carried, headers());
            assert_eq!(limit, MAX_FAL_REPLY_BYTES);
        }
        other => panic!("{other:?}"),
    }
    let mut at_bound = vec![b' '; MAX_FAL_REPLY_BYTES];
    at_bound[..12].copy_from_slice(br#"{"image":{}}"#);
    assert_eq!(appended_at(&at_bound).data, Some(Vec::new()));
}

fn appended_at(body: &[u8]) -> ImageResponse {
    let mut response = ImageResponse::default();
    append_singular_image(&mut response, 200, &headers(), body).expect("a reply at the limit");
    response
}

// the fuzz target feeds these seeds to the same entry point, so a panic here is a panic there
#[rstest]
fn replays_the_bria_image_corpus(#[files("fuzz/corpus/fal_image_reply/*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("a corpus file");
    let mut response = ImageResponse::default();
    drop(append_singular_image(
        &mut response,
        200,
        &headers(),
        &bytes,
    ));
    if path
        .file_name()
        .is_some_and(|name| name == "bria_singular-image.json")
    {
        assert_eq!(response.data, Some(vec![url_image("https://a.test/1.png")]));
    }
}
