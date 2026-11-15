#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::{
    FluxError, FluxImageResponse, FluxProfile, SUPPORTED_OPENAI_PARAMS, map_aspect_ratio,
    map_image_size, project_response,
};
use crate::api::fal::images::decode::{
    FalImage, FalImageProviderSpecificFields, MAX_FAL_REPLY_BYTES,
};
use crate::api::fal::images::error::FalImageError;
use crate::api::fal::images::params::image_generation_body;

const MODEL: &str = "fal-ai/flux/dev";

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
    profile: FluxProfile,
    non_default: &[(&str, Json)],
    optional: &[(&str, Json)],
) -> ImageParams {
    profile
        .map_openai_params(&params(non_default), params(optional), MODEL, false)
        .expect("mapped parameters")
}

fn reply_headers() -> ImageHeaders {
    ImageHeaders::from([("x-request-id".to_owned(), "req-1".to_owned())])
}

fn projected(status: u16, body: &str) -> FluxImageResponse {
    let mut response = FluxImageResponse::default();
    project_response(status, &reply_headers(), body.as_bytes(), &mut response)
        .expect("a projected reply");
    response
}

#[test]
fn flux_profiles_keep_their_endpoints() {
    assert_eq!(FluxProfile::Ultra.endpoint(), "fal-ai/flux-pro/v1.1-ultra");
    assert_eq!(FluxProfile::ProV11.endpoint(), "fal-ai/flux-pro/v1.1");
    assert_eq!(FluxProfile::Schnell.endpoint(), "fal-ai/flux/schnell");
    assert_eq!(FluxProfile::Dev.endpoint(), "fal-ai/flux/dev");
    assert_eq!(FluxProfile::Ultra.size_parameter(), "aspect_ratio");
    assert_eq!(FluxProfile::ProV11.size_parameter(), "image_size");
    assert_eq!(FluxProfile::Schnell.size_parameter(), "image_size");
    assert_eq!(FluxProfile::Dev.size_parameter(), "image_size");
}

#[test]
fn flux_profiles_advertise_the_three_openai_params() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["n", "response_format", "size"]);
}

#[test]
fn flux_n_maps_to_num_images_under_the_original_key_rule() {
    assert_eq!(
        mapped(FluxProfile::Ultra, &[("n", number(2))], &[]),
        params(&[("num_images", number(2))])
    );
    assert_eq!(
        mapped(
            FluxProfile::Dev,
            &[("n", number(3))],
            &[("num_images", number(1))]
        ),
        params(&[("num_images", number(3))])
    );
    assert_eq!(
        mapped(
            FluxProfile::Schnell,
            &[("n", number(2))],
            &[("n", number(5))]
        ),
        params(&[("n", number(5))])
    );
}

#[rstest]
#[case::url("url")]
#[case::b64_json("b64_json")]
fn flux_response_format_url_and_b64_json_become_jpeg(#[case] format: &str) {
    assert_eq!(
        mapped(
            FluxProfile::ProV11,
            &[("response_format", text(format))],
            &[]
        ),
        params(&[("output_format", text("jpeg"))])
    );
    assert_eq!(
        mapped(FluxProfile::Ultra, &[("response_format", text("png"))], &[]),
        params(&[("output_format", text("png"))])
    );
    assert_eq!(
        mapped(
            FluxProfile::Ultra,
            &[("response_format", text("url"))],
            &[("output_format", text("png"))]
        ),
        params(&[("output_format", text("jpeg"))])
    );
    assert_eq!(
        mapped(
            FluxProfile::Ultra,
            &[("response_format", text("url"))],
            &[("response_format", text("png"))]
        ),
        params(&[("response_format", text("png"))])
    );
}

#[test]
fn flux_unsupported_parameters_fail_unless_dropped() {
    let error = FluxProfile::Ultra
        .map_openai_params(
            &params(&[("seed", number(1))]),
            ImageParams::new(),
            MODEL,
            false,
        )
        .expect_err("an unsupported parameter");
    assert!(matches!(
        error,
        FluxError::UnsupportedParameter { parameter, ref model } if parameter == "seed" && model == MODEL
    ));
    assert_eq!(
        FluxProfile::Ultra
            .map_openai_params(
                &params(&[("seed", number(1))]),
                ImageParams::new(),
                MODEL,
                true
            )
            .expect("dropped parameters"),
        ImageParams::new()
    );
    assert_eq!(
        mapped(
            FluxProfile::Ultra,
            &[("seed", number(1))],
            &[("seed", number(7))]
        ),
        params(&[("seed", number(7))])
    );
}

#[test]
fn flux_request_body_overlays_provider_fields_on_prompt() {
    let body = image_generation_body(
        "a cat",
        mapped(
            FluxProfile::Ultra,
            &[("size", text("1500x1000")), ("n", number(2))],
            &[],
        ),
    );
    assert_eq!(
        body,
        params(&[
            ("prompt", text("a cat")),
            ("aspect_ratio", text("4:3")),
            ("num_images", number(2))
        ])
    );
    let overlaid = image_generation_body(
        "a cat",
        mapped(FluxProfile::Dev, &[], &[("prompt", text("a dog"))]),
    );
    assert_eq!(overlaid, params(&[("prompt", text("a dog"))]));
}

#[rstest]
#[case("1024x1024", "1:1")]
#[case("512x512", "1:1")]
#[case("1792x1024", "16:9")]
#[case("1024x1792", "9:16")]
#[case("1024x768", "4:3")]
#[case("768x1024", "3:4")]
#[case("1536x1024", "3:2")]
#[case("1024x1536", "2:3")]
#[case("2048x876", "21:9")]
#[case("876x2048", "9:21")]
fn flux_ultra_exact_sizes_use_their_aliases(#[case] size: &str, #[case] aspect_ratio: &str) {
    assert_eq!(map_aspect_ratio(size), aspect_ratio);
}

#[test]
fn flux_ultra_exact_and_custom_sizes_differ_across_overlapping_intervals() {
    assert_eq!(map_aspect_ratio("1536x1024"), "3:2");
    assert_eq!(map_aspect_ratio("1500x1000"), "4:3");
    assert_eq!(map_aspect_ratio("1400x1000"), "4:3");
    assert_eq!(map_aspect_ratio("1499x1000"), "4:3");
    assert_eq!(map_aspect_ratio("1599x1000"), "4:3");
    assert_eq!(map_aspect_ratio("1600x1000"), "4:3");
}

#[rstest]
#[case("499x1000", "9:21")]
#[case("500x1000", "9:16")]
#[case("599x1000", "9:16")]
#[case("600x1000", "3:4")]
#[case("649x1000", "3:4")]
#[case("650x1000", "3:4")]
#[case("699x1000", "3:4")]
#[case("700x1000", "2:3")]
#[case("749x1000", "2:3")]
#[case("750x1000", "16:9")]
#[case("949x1000", "16:9")]
#[case("950x1000", "1:1")]
#[case("1000x1000", "1:1")]
#[case("1050x1000", "1:1")]
#[case("1051x1000", "16:9")]
#[case("1299x1000", "16:9")]
#[case("1300x1000", "4:3")]
#[case("1699x1000", "4:3")]
#[case("1700x1000", "16:9")]
#[case("2299x1000", "16:9")]
#[case("2300x1000", "21:9")]
#[case("2301x1000", "21:9")]
fn flux_ultra_ratio_intervals_keep_their_boundaries(
    #[case] size: &str,
    #[case] aspect_ratio: &str,
) {
    assert_eq!(map_aspect_ratio(size), aspect_ratio);
}

#[test]
fn flux_ultra_ratio_intervals_compare_exactly_past_doubles() {
    // this ratio is 1.7 minus one part in 1e16, which an IEEE double rounds up to 1.7
    assert_eq!(
        map_aspect_ratio("16999999999999999x10000000000000000"),
        "4:3"
    );
}

#[test]
fn flux_ultra_portrait_overlap_prefers_the_first_interval() {
    assert_eq!(map_aspect_ratio("650x1000"), "3:4");
    assert_eq!(map_aspect_ratio("660x1000"), "3:4");
    assert_eq!(map_aspect_ratio("700x1000"), "2:3");
    assert_eq!(map_aspect_ratio("0x100"), "9:21");
    assert_eq!(map_aspect_ratio("-1024x768"), "9:21");
    assert_eq!(map_aspect_ratio("-1024x-768"), "4:3");
    assert_eq!(map_aspect_ratio("1_0x2_0"), "9:16");
}

#[rstest]
#[case("1024x0")]
#[case("0x0")]
#[case("axb")]
#[case("1024X768")]
#[case("auto")]
#[case("")]
#[case("1024x768x1")]
#[case("1024-x-768")]
#[case("x")]
#[case("99999999999999999999x1000")]
#[case("١٠٢٤x٧٦٨")]
fn flux_ultra_invalid_sizes_fall_back_to_sixteen_nine(#[case] size: &str) {
    assert_eq!(map_aspect_ratio(size), "16:9");
}

#[rstest]
#[case::number(number(7))]
#[case::bool(Json::Bool(true))]
#[case::null(Json::Null)]
#[case::list(Json::List(Vec::new()))]
fn flux_ultra_size_must_be_text(#[case] size: Json) {
    let error = FluxProfile::Ultra
        .map_openai_params(&params(&[("size", size)]), ImageParams::new(), MODEL, false)
        .expect_err("a non-text size");
    assert!(matches!(error, FluxError::SizeNotText));
}

#[rstest]
#[case("1024x1024", "square_hd")]
#[case("512x512", "square")]
#[case("1792x1024", "landscape_16_9")]
#[case("1024x1792", "portrait_16_9")]
#[case("1024x768", "landscape_4_3")]
#[case("768x1024", "portrait_4_3")]
fn flux_image_sizes_use_their_aliases(#[case] size: &str, #[case] image_size: &str) {
    assert_eq!(
        mapped(FluxProfile::ProV11, &[("size", text(size))], &[]),
        params(&[("image_size", text(image_size))])
    );
    assert_eq!(
        mapped(FluxProfile::Schnell, &[("size", text(size))], &[]),
        params(&[("image_size", text(image_size))])
    );
    assert_eq!(
        mapped(FluxProfile::Dev, &[("size", text(size))], &[]),
        params(&[("image_size", text(image_size))])
    );
}

#[rstest]
#[case::object(Json::Object(IndexMap::from([("width".to_owned(), number(512))])))]
#[case::number(number(7))]
#[case::bool(Json::Bool(true))]
#[case::null(Json::Null)]
#[case::list(Json::List(vec![number(1)]))]
#[case::float(Json::Number(Number::from_f64(1.5).expect("a finite number")))]
fn flux_image_size_passes_non_strings_through(#[case] size: Json) {
    assert_eq!(
        mapped(FluxProfile::ProV11, &[("size", size.clone())], &[]),
        params(&[("image_size", size.clone())])
    );
    assert_eq!(
        mapped(FluxProfile::Schnell, &[("size", size.clone())], &[]),
        params(&[("image_size", size.clone())])
    );
    assert_eq!(
        mapped(FluxProfile::Dev, &[("size", size.clone())], &[]),
        params(&[("image_size", size.clone())])
    );
    assert_eq!(map_image_size(&size), size);
}

#[rstest]
#[case("1500x1000", 1500, 1000)]
#[case("1024x0", 1024, 0)]
#[case("-5x10", -5, 10)]
#[case(" 10x20 ", 10, 20)]
#[case("1_0x2_0", 10, 20)]
#[case("+10x20", 10, 20)]
fn flux_image_size_custom_dimensions_become_objects(
    #[case] size: &str,
    #[case] width: i64,
    #[case] height: i64,
) {
    let expected = Json::Object(IndexMap::from([
        ("width".to_owned(), number(width)),
        ("height".to_owned(), number(height)),
    ]));
    assert_eq!(map_image_size(&text(size)), expected);
}

#[rstest]
#[case("auto")]
#[case("1024X768")]
#[case("10x20x30")]
#[case("")]
#[case("1024-x-768")]
#[case("10x")]
#[case("99999999999999999999x1000")]
#[case("١٠٢٤x٧٦٨")]
fn flux_image_size_invalid_sizes_fall_back_to_landscape(#[case] size: &str) {
    assert_eq!(map_image_size(&text(size)), text("landscape_4_3"));
}

#[test]
fn flux_projection_appends_image_objects() {
    let body = r#"{"images": ["https://a.test/1.png", {"url": "https://b.test/2.png", "b64_json": "aGk=", "width": 640, "height": 480, "content_type": "image/webp"}, 7, null, {"url": 5}]}"#;
    let response = projected(200, body);
    assert_eq!(
        response.data,
        vec![
            FalImage {
                url: Some("https://a.test/1.png".to_owned()),
                b64_json: None,
                provider_specific_fields: None,
            },
            FalImage {
                url: Some("https://b.test/2.png".to_owned()),
                b64_json: Some("aGk=".to_owned()),
                provider_specific_fields: Some(FalImageProviderSpecificFields {
                    width: Some(640),
                    height: Some(480),
                    content_type: Some("image/webp".to_owned()),
                }),
            },
            FalImage {
                url: None,
                b64_json: None,
                provider_specific_fields: None,
            },
        ]
    );
    assert!(response.hidden_params.is_empty());
}

#[test]
fn flux_projection_copies_present_hidden_metadata() {
    let response = projected(
        200,
        r#"{"images": [], "seed": 42, "timings": {"inference": 2.5}, "has_nsfw_concepts": [false], "prompt": "ignored"}"#,
    );
    assert_eq!(
        response.hidden_params,
        IndexMap::from([
            ("seed".to_owned(), number(42)),
            (
                "timings".to_owned(),
                Json::Object(IndexMap::from([(
                    "inference".to_owned(),
                    Json::Number(Number::from_f64(2.5).expect("a finite number")),
                )])),
            ),
            (
                "has_nsfw_concepts".to_owned(),
                Json::List(vec![Json::Bool(false)]),
            ),
        ])
    );
    let retained = projected(200, r#"{"images": [], "seed": null, "timings": null}"#);
    assert_eq!(
        retained.hidden_params,
        IndexMap::from([
            ("seed".to_owned(), Json::Null),
            ("timings".to_owned(), Json::Null),
        ])
    );
}

#[test]
fn flux_projection_keeps_existing_hidden_metadata_and_data() {
    let mut response = FluxImageResponse {
        data: vec![FalImage {
            url: Some("https://existing.test/0.png".to_owned()),
            b64_json: None,
            provider_specific_fields: None,
        }],
        hidden_params: IndexMap::from([("request-id".to_owned(), text("req-1"))]),
    };
    project_response(
        200,
        &reply_headers(),
        br#"{"images": ["https://a.test/1.png"], "seed": null}"#,
        &mut response,
    )
    .expect("a projected reply");
    assert_eq!(
        response.hidden_params,
        IndexMap::from([
            ("request-id".to_owned(), text("req-1")),
            ("seed".to_owned(), Json::Null),
        ])
    );
    assert_eq!(response.data.len(), 2);
    assert_eq!(
        response.data[0].url.as_deref(),
        Some("https://existing.test/0.png")
    );
    assert_eq!(
        response.data[1].url.as_deref(),
        Some("https://a.test/1.png")
    );
}

#[test]
fn flux_projection_keeps_reply_errors_with_status_and_headers() {
    let mut response = FluxImageResponse::default();
    let error =
        project_response(502, &reply_headers(), b"{", &mut response).expect_err("a broken reply");
    assert!(matches!(
        error,
        FalImageError::ReplyDecode { status: 502, headers, .. } if headers == reply_headers()
    ));
    let list_error =
        project_response(200, &reply_headers(), b"[]", &mut response).expect_err("a list reply");
    assert!(matches!(
        list_error,
        FalImageError::ReplyNotAnObject { status: 200, headers } if headers == reply_headers()
    ));
    assert!(response.data.is_empty());
    assert!(
        projected(200, r#"{"images": {"url": "not a list"}}"#)
            .data
            .is_empty()
    );
}

#[test]
fn flux_projection_refuses_a_reply_past_the_size_limit() {
    let body = format!(
        r#"{{"images": [], "prompt": "{}"}}"#,
        "a".repeat(MAX_FAL_REPLY_BYTES)
    );
    let mut response = FluxImageResponse::default();
    let error = project_response(200, &reply_headers(), body.as_bytes(), &mut response)
        .expect_err("an oversized reply");
    assert!(matches!(
        error,
        FalImageError::ReplyTooLarge { status: 200, headers, limit }
            if headers == reply_headers() && limit == MAX_FAL_REPLY_BYTES
    ));
}
