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
use super::append_url_images;
use super::map_openai_params;
use super::size::{SQUARE_HD, map_image_size};
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

fn dimensions(width: i64, height: i64) -> Json {
    Json::Object(IndexMap::from([
        ("width".to_owned(), number(width)),
        ("height".to_owned(), number(height)),
    ]))
}

fn url_image(url: Option<&str>) -> ImageData {
    ImageData {
        b64_json: None,
        revised_prompt: None,
        url: url.map(str::to_owned),
    }
}

fn appended(body: &str) -> ImageResponse {
    let mut response = ImageResponse::default();
    append_url_images(&mut response, 200, &headers(), body.as_bytes()).expect("a reply");
    response
}

#[rstest]
#[case::square_hd("1024x1024", "square_hd")]
#[case::square("512x512", "square")]
#[case::portrait_four_thirds("768x1024", "portrait_4_3")]
#[case::portrait_sixteen_ninths("576x1024", "portrait_16_9")]
#[case::landscape_four_thirds("1024x768", "landscape_4_3")]
#[case::landscape_sixteen_ninths("1024x576", "landscape_16_9")]
fn recraft_presets_map_to_their_names(#[case] size: &str, #[case] preset: &str) {
    assert_eq!(map_image_size(size), text(preset));
}

#[rstest]
#[case::openai_landscape("1792x1024", 1792, 1024)]
#[case::native_landscape("1280x960", 1280, 960)]
#[case::native_portrait("960x1280", 960, 1280)]
#[case::spaced(" 1024x 768", 1024, 768)]
#[case::underscored("1_0x20", 10, 20)]
#[case::signed("-5x+6", -5, 6)]
#[case::zero("0x0", 0, 0)]
fn recraft_custom_dimensions_become_integer_objects(
    #[case] size: &str,
    #[case] width: i64,
    #[case] height: i64,
) {
    assert_eq!(map_image_size(size), dimensions(width, height));
}

#[rstest]
#[case::auto("auto")]
#[case::empty("")]
#[case::upper_x("1024X1024")]
#[case::fraction("1.5x2")]
#[case::fal_separator("10-x-20")]
#[case::three_parts("1x2x3")]
#[case::trailing_x("1024x1024x")]
#[case::words("axb")]
#[case::past_i64("999999999999999999999999x1")]
#[case::min_i64("-9223372036854775808x20")]
#[case::non_ascii_digits("١٠٢٤x٧٦٨")]
fn recraft_unreadable_sizes_become_square_hd(#[case] size: &str) {
    assert_eq!(map_image_size(size), text(SQUARE_HD));
}

#[test]
fn recraft_mapping_ignores_n_and_response_format() {
    let non_default = params(&[
        ("n", number(3)),
        ("response_format", text("url")),
        ("size", text("1024x768")),
    ]);
    let mapped = map_openai_params(&non_default, ImageParams::new(), ENDPOINT, false)
        .expect("ignored options");
    assert_eq!(mapped, params(&[("image_size", text("landscape_4_3"))]));
}

#[test]
fn recraft_mapping_keeps_an_existing_original_option() {
    let non_default = params(&[("size", text("1024x768"))]);
    let optional = params(&[("size", text("auto"))]);
    let mapped = map_openai_params(&non_default, optional, ENDPOINT, false)
        .expect("an existing original option");
    assert_eq!(mapped, params(&[("size", text("auto"))]));
}

#[test]
fn recraft_mapping_overwrites_the_translated_destination() {
    let non_default = params(&[("size", text("1792x1024"))]);
    let optional = params(&[
        ("aspect_ratio", text("3:2")),
        ("image_size", text("square")),
    ]);
    let mapped = map_openai_params(&non_default, optional, ENDPOINT, false)
        .expect("a translated destination");
    assert_eq!(
        mapped,
        params(&[
            ("aspect_ratio", text("3:2")),
            ("image_size", dimensions(1792, 1024)),
        ])
    );
}

#[test]
fn recraft_mapping_rejects_the_first_unsupported_option() {
    let non_default = params(&[("seed", number(1)), ("user", text("alice"))]);
    match map_openai_params(&non_default, ImageParams::new(), ENDPOINT, false) {
        Err(FalImageError::UnsupportedParameters { parameters, model }) => {
            assert_eq!(parameters, ["seed"]);
            assert_eq!(model, ENDPOINT);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn recraft_mapping_drops_unsupported_options_with_drop_params() {
    let non_default = params(&[("seed", number(1)), ("n", number(2))]);
    let mapped = map_openai_params(&non_default, ImageParams::new(), ENDPOINT, true)
        .expect("dropped options");
    assert!(mapped.is_empty());
}

#[test]
fn recraft_mapping_keeps_an_unsupported_option_already_present() {
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
fn recraft_mapping_requires_a_text_size(#[case] size: Json) {
    let non_default = params(&[("size", size)]);
    match map_openai_params(&non_default, ImageParams::new(), ENDPOINT, false) {
        Err(FalImageError::SizeNotText) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn recraft_url_list_appends_every_url_member() {
    let response = appended(
        r#"{"images": ["https://a.test/1.png", {"url": "https://b.test/2.png", "b64_json": "aGk=", "width": 10, "height": 20, "content_type": "image/webp", "seed": 7}, 7, null, true, [], {}]}"#,
    );
    assert_eq!(
        response.data,
        Some(vec![
            url_image(Some("https://a.test/1.png")),
            url_image(Some("https://b.test/2.png")),
            url_image(None),
        ])
    );
}

#[rstest]
#[case::missing("{}")]
#[case::singular(r#"{"image": {"url": "https://a.test/1.png"}}"#)]
#[case::object(r#"{"images": {"url": "https://a.test/1.png"}}"#)]
#[case::null(r#"{"images": null}"#)]
#[case::text(r#"{"images": "https://a.test/1.png"}"#)]
#[case::empty_list(r#"{"images": []}"#)]
fn recraft_reply_without_a_list_appends_nothing(#[case] body: &str) {
    assert_eq!(appended(body).data, Some(Vec::new()));
}

#[test]
fn recraft_map_member_without_a_url_keeps_a_null_url() {
    assert_eq!(
        appended(r#"{"images": [{"url": null}]}"#).data,
        Some(vec![url_image(None)])
    );
    assert_eq!(
        appended(r#"{"images": [{}]}"#).data,
        Some(vec![url_image(None)])
    );
}

#[rstest]
#[case::number(r#"{"images": [{"url": 5}]}"#)]
#[case::truth(r#"{"images": [{"url": true}]}"#)]
#[case::list(r#"{"images": [{"url": []}]}"#)]
#[case::object(r#"{"images": [{"url": {}}]}"#)]
fn recraft_image_url_must_be_text(#[case] body: &str) {
    let mut response = ImageResponse::default();
    match append_url_images(&mut response, 200, &headers(), body.as_bytes()) {
        Err(FalImageError::FieldNotText { field, model }) => {
            assert_eq!(field, "url");
            assert_eq!(model, ENDPOINT);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn recraft_reply_keeps_existing_images() {
    let mut response = ImageResponse {
        created: 0,
        data: Some(vec![url_image(Some("https://first.test/0.png"))]),
        usage: None,
    };
    append_url_images(
        &mut response,
        200,
        &headers(),
        br#"{"images": ["https://a.test/1.png"]}"#,
    )
    .expect("a url list");
    assert_eq!(
        response.data,
        Some(vec![
            url_image(Some("https://first.test/0.png")),
            url_image(Some("https://a.test/1.png")),
        ])
    );
}

#[test]
fn recraft_malformed_reply_keeps_the_source_status_and_headers() {
    let mut response = ImageResponse::default();
    match append_url_images(&mut response, 502, &headers(), b"{") {
        Err(FalImageError::ReplyDecode {
            status,
            headers: carried,
            ..
        }) => {
            assert_eq!(status, 502);
            assert_eq!(carried, headers());
        }
        other => panic!("{other:?}"),
    }
}

#[rstest]
#[case::list(b"[]")]
#[case::text(br#""images""#)]
#[case::number(b"7")]
#[case::null(b"null")]
fn recraft_reply_requires_an_object_root(#[case] body: &[u8]) {
    let mut response = ImageResponse::default();
    match append_url_images(&mut response, 500, &headers(), body) {
        Err(FalImageError::ReplyNotAnObject {
            status,
            headers: carried,
        }) => {
            assert_eq!(status, 500);
            assert_eq!(carried, headers());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn recraft_reply_size_is_bounded() {
    let oversized = vec![b' '; MAX_FAL_REPLY_BYTES + 1];
    let mut response = ImageResponse::default();
    match append_url_images(&mut response, 413, &headers(), &oversized) {
        Err(FalImageError::ReplyTooLarge {
            status,
            headers: carried,
            limit,
        }) => {
            assert_eq!(status, 413);
            assert_eq!(carried, headers());
            assert_eq!(limit, MAX_FAL_REPLY_BYTES);
        }
        other => panic!("{other:?}"),
    }
    let mut at_bound = vec![b' '; MAX_FAL_REPLY_BYTES];
    at_bound[..13].copy_from_slice(br#"{"images":[]}"#);
    let mut response = ImageResponse::default();
    append_url_images(&mut response, 200, &headers(), &at_bound).expect("a reply at the limit");
    assert_eq!(response.data, Some(Vec::new()));
}

// the fuzz target feeds these seeds to the same entry point, so a panic here is a panic there
#[rstest]
fn replays_the_recraft_image_corpus(#[files("fuzz/corpus/fal_image_reply/*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("a corpus file");
    let mut response = ImageResponse::default();
    drop(append_url_images(&mut response, 200, &headers(), &bytes));
    if path
        .file_name()
        .is_some_and(|name| name == "mixed-list.json")
    {
        assert_eq!(
            response.data,
            Some(vec![
                url_image(Some("https://a.test/1.png")),
                url_image(Some("https://b.test/2.png")),
                url_image(None),
            ])
        );
    }
}
