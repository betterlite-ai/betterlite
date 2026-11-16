#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::{
    decode_video_id_with_provider, encode_video_id_with_provider,
};
use betterlite_openai::videos::reply::{VideoObject, VideoObjectType};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::VideoHeaders;
use super::VideoParams;
use super::create::{CreateResponseInput, FAL_AI_PROVIDER, create_request, create_response};
use super::decode::{
    FalVideoDetail, FalVideoDetailItem, FalVideoLocation, FalVideoReply, FalVideoResult,
    MAX_FAL_VIDEO_REPLY_BYTES, decode_fal_video_reply,
};
use super::duration::{DurationValue, duration_value, numeric_duration};
use super::error::{FalVideoError, get_error_class};
use super::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::paths::{
    model_path_from_request_url, queue_request_base_path, request_id_from_request_url,
};
use super::profile::{
    H3_PROFILE, SEEDANCE_PROFILE, VideoProfile, profile_for_model, resolution_for_short_side,
};
use super::settings::{DEFAULT_QUEUE_BASE, FalVideoSettings, complete_url};
use super::size::{ALLOWED_ASPECT_RATIOS, size_params};
use super::urls::{decode_video_id, job_urls};

const H3_MODEL: &str = "minimax/h3/hailuo-02";
const DEFAULT_MODEL: &str = "fal-ai/kling-video";
const NOW: i64 = 1_767_225_600;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: &str) -> Json {
    Json::Number(value.parse().expect("a number"))
}

fn params(pairs: &[(&str, Json)]) -> VideoParams {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn reply(body: &str) -> FalVideoReply {
    decode_fal_video_reply(200, &VideoHeaders::new(), body.as_bytes()).expect("a reply")
}

fn usage_text(video: &VideoObject, key: &str) -> Option<String> {
    let usage = serde_json::to_value(video.usage.as_ref()).ok()?;
    let field = usage.get(key)?;
    Some(
        field
            .as_str()
            .map_or_else(|| field.to_string(), str::to_owned),
    )
}

#[rstest]
#[case::auto(text("auto"), Some(DurationValue::Auto))]
#[case::integer(text("2"), Some(DurationValue::Numeric("2".to_owned())))]
#[case::rounds_toward_zero(text("2.9"), Some(DurationValue::Numeric("2".to_owned())))]
#[case::negative_truncates(text("-2.9"), Some(DurationValue::Numeric("-2".to_owned())))]
#[case::negative_below_one(text("-0.5"), Some(DurationValue::Numeric("0".to_owned())))]
#[case::whitespace(text(" 3 "), Some(DurationValue::Numeric("3".to_owned())))]
#[case::plus_sign(text("+7"), Some(DurationValue::Numeric("7".to_owned())))]
#[case::trailing_dot(text("5."), Some(DurationValue::Numeric("5".to_owned())))]
#[case::leading_dot(text(".5"), Some(DurationValue::Numeric("0".to_owned())))]
#[case::exponent(text("1e3"), Some(DurationValue::Numeric("1000".to_owned())))]
#[case::negative_exponent(text("1E-3"), Some(DurationValue::Numeric("0".to_owned())))]
#[case::exponent_shift(text("1.5e1"), Some(DurationValue::Numeric("15".to_owned())))]
#[case::trailing_zeros(text("2.50"), Some(DurationValue::Numeric("2".to_owned())))]
#[case::leading_zeros(text("0009"), Some(DurationValue::Numeric("9".to_owned())))]
#[case::number_integer(number("12"), Some(DurationValue::Numeric("12".to_owned())))]
#[case::number_float(number("2.9"), Some(DurationValue::Numeric("2".to_owned())))]
#[case::too_many_digits(text("1e5000"), None)]
#[case::zero_huge_exponent(text("0e5000"), Some(DurationValue::Numeric("0".to_owned())))]
#[case::zero_huge_exponent_past_i64(text("0e99999999999999999999"), Some(DurationValue::Numeric("0".to_owned())))]
#[case::tiny_past_i64_exponent(text("1e-99999999999999999999"), Some(DurationValue::Numeric("0".to_owned())))]
#[case::tiny_exponent_at_the_i64_edge(text("1.55e-9223372036854775807"), Some(DurationValue::Numeric("0".to_owned())))]
#[case::too_large_exponent(text("1e99999999999999999999"), None)]
#[case::exponent_at_the_positive_edge(text("1e9223372036854775807"), None)]
#[case::empty_zero_exponent(text("0e"), None)]
#[case::non_digit_exponent(text("1e-abc"), None)]
#[case::boolean(Json::Bool(true), None)]
#[case::null(Json::Null, None)]
#[case::word(text("abc"), None)]
#[case::empty(text(""), None)]
#[case::nan(text("nan"), None)]
#[case::infinity(text("inf"), None)]
#[case::underscored(text("1_0"), None)]
#[case::double_dot(text("1.2.3"), None)]
#[case::empty_exponent(text("1e"), None)]
#[case::object(Json::Object(IndexMap::new()), None)]
#[case::list(Json::List(Vec::new()), None)]
fn duration_reads_exact_integers(#[case] value: Json, #[case] expected: Option<DurationValue>) {
    assert_eq!(duration_value(&value), expected);
}

#[rstest]
#[case::past_double("9007199254740993", "9007199254740993")]
#[case::far_past_double("123456789012345678901234567890", "123456789012345678901234567890")]
#[case::tiny_fraction("0.9999999999999999999999999999", "0")]
fn duration_keeps_more_than_a_double_can(#[case] raw: &str, #[case] expected: &str) {
    assert_eq!(
        duration_value(&text(raw)),
        Some(DurationValue::Numeric(expected.to_owned()))
    );
}

#[test]
fn the_digit_cap_keeps_4096_digits_and_drops_4097() {
    let zeros = "0".repeat(4095);
    let expected = format!("1{zeros}");
    assert_eq!(
        duration_value(&text("1e4095")),
        Some(DurationValue::Numeric(expected))
    );
    assert_eq!(duration_value(&text("1e4096")), None);
}

#[test]
fn leading_zeros_do_not_count_toward_the_digit_cap() {
    let zeros = "0".repeat(4096);
    let raw = format!("{zeros}5");
    assert_eq!(
        duration_value(&text(&raw)),
        Some(DurationValue::Numeric("5".to_owned()))
    );
}

#[rstest]
#[case::numeric(Some(text("2.5")), Some("2"))]
#[case::auto(Some(text("auto")), None)]
#[case::absent(None, None)]
#[case::boolean(Some(Json::Bool(false)), None)]
fn numeric_duration_skips_auto_and_missing(
    #[case] value: Option<Json>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(numeric_duration(value.as_ref()).as_deref(), expected);
}

#[rstest]
#[case::h3("minimax/h3/hailuo-02", true)]
#[case::h3_uppercase("minimax/H3/hailuo-02", false)]
#[case::h3_without_slash("minimax/h3", false)]
#[case::h3_inside_another_path("fal-ai/minimax/h3/x", false)]
#[case::empty("", false)]
fn profile_selection_is_a_case_sensitive_prefix(#[case] model: &str, #[case] h3: bool) {
    let profile = profile_for_model(model);
    assert_eq!(profile.integer_duration, h3);
    assert_eq!(profile.reference_as_list, h3);
    assert_eq!(
        profile.reference_key,
        if h3 {
            "reference_image_urls"
        } else {
            "image_url"
        }
    );
}

#[rstest]
#[case::h3_480(&H3_PROFILE, 480, "480P")]
#[case::h3_481(&H3_PROFILE, 481, "768P")]
#[case::h3_768(&H3_PROFILE, 768, "768P")]
#[case::h3_769(&H3_PROFILE, 769, "2K")]
#[case::h3_1440(&H3_PROFILE, 1440, "2K")]
#[case::h3_1441(&H3_PROFILE, 1441, "4K")]
#[case::h3_max(&H3_PROFILE, u128::MAX, "4K")]
#[case::seedance_zero(&SEEDANCE_PROFILE, 0, "480p")]
#[case::seedance_480(&SEEDANCE_PROFILE, 480, "480p")]
#[case::seedance_481(&SEEDANCE_PROFILE, 481, "720p")]
#[case::seedance_720(&SEEDANCE_PROFILE, 720, "720p")]
#[case::seedance_721(&SEEDANCE_PROFILE, 721, "1080p")]
#[case::seedance_1080(&SEEDANCE_PROFILE, 1080, "1080p")]
#[case::seedance_1081(&SEEDANCE_PROFILE, 1081, "4k")]
fn short_side_tiers_are_inclusive(
    #[case] profile: &VideoProfile,
    #[case] short_side: u128,
    #[case] expected: &str,
) {
    assert_eq!(resolution_for_short_side(short_side, profile), expected);
}

#[rstest]
#[case::seedance_lower("720p", &SEEDANCE_PROFILE, Some("720p"))]
#[case::seedance_upper("720P", &SEEDANCE_PROFILE, Some("720p"))]
#[case::seedance_4k_upper("4K", &SEEDANCE_PROFILE, Some("4k"))]
#[case::h3_upper("2K", &H3_PROFILE, Some("2K"))]
#[case::h3_lower("2k", &H3_PROFILE, Some("2K"))]
#[case::h3_480_lower("480p", &H3_PROFILE, Some("480P"))]
#[case::h3_1080_missing("1080P", &H3_PROFILE, None)]
#[case::auto_is_not_a_resolution("auto", &SEEDANCE_PROFILE, None)]
fn named_resolutions_map_case_insensitively(
    #[case] raw: &str,
    #[case] profile: &VideoProfile,
    #[case] expected: Option<&str>,
) {
    let mapped = size_params(&text(raw), profile);
    let expected_resolution = expected.map(text);
    assert_eq!(mapped.get("resolution"), expected_resolution.as_ref());
    assert_eq!(mapped.contains_key("resolution"), expected.is_some());
}

fn assert_aspect(raw: &str, profile: &VideoProfile, resolution: &str, aspect: Option<&str>) {
    let mapped = size_params(&text(raw), profile);
    assert_eq!(mapped.get("resolution"), Some(&text(resolution)));
    assert_eq!(mapped.get("aspect_ratio"), aspect.map(text).as_ref());
    assert_eq!(mapped.contains_key("aspect_ratio"), aspect.is_some());
}

#[rstest]
#[case::four_three("1024x768", "1080p", Some("4:3"))]
#[case::wide("1280x720", "720p", Some("16:9"))]
#[case::square("1000x1000", "1080p", Some("1:1"))]
#[case::three_colon_two("300x200", "480p", None)]
#[case::tall("3x4", "480p", Some("3:4"))]
#[case::cinema("21x9", "480p", None)]
#[case::narrow("9x16", "480p", Some("9:16"))]
#[case::two_to_one("2x1", "480p", None)]
#[case::upper_x("10X20", "480p", None)]
#[case::max_width("340282366920938463463374607431768211455x1", "480p", None)]
fn sizes_reduce_the_ratio_and_pick_the_short_side_band(
    #[case] raw: &str,
    #[case] resolution: &str,
    #[case] aspect: Option<&str>,
) {
    assert_aspect(raw, &SEEDANCE_PROFILE, resolution, aspect);
}

#[rstest]
#[case::square("1000x1000", "2K", Some("1:1"))]
#[case::two_to_one("2000x1000", "2K", None)]
#[case::small_square("400x400", "480P", Some("1:1"))]
fn h3_sizes_use_the_h3_bands(
    #[case] raw: &str,
    #[case] resolution: &str,
    #[case] aspect: Option<&str>,
) {
    assert_aspect(raw, &H3_PROFILE, resolution, aspect);
}

#[rstest]
#[case::two_x("10x20x30")]
#[case::missing_width("x10")]
#[case::missing_height("10x")]
#[case::zero_width("0x10")]
#[case::zero_height("10x0")]
#[case::spaces(" 10x20")]
#[case::signs("-10x20")]
#[case::plus_width("+10x20")]
#[case::plus_height("10x+20")]
#[case::unicode_digits("\u{661}\u{662}x\u{663}\u{664}")]
#[case::overflow("340282366920938463463374607431768211456x2")]
#[case::empty("")]
fn malformed_sizes_map_no_params(#[case] raw: &str) {
    assert!(size_params(&text(raw), &SEEDANCE_PROFILE).is_empty());
}

#[rstest]
#[case::number(number("1024"))]
#[case::boolean(Json::Bool(true))]
#[case::null(Json::Null)]
#[case::list(Json::List(vec![text("10x20")]))]
#[case::object(Json::Object(IndexMap::new()))]
fn non_text_sizes_map_no_params(#[case] size: Json) {
    assert!(size_params(&size, &SEEDANCE_PROFILE).is_empty());
}

#[test]
fn the_aspect_allowlist_holds_the_seven_declared_ratios() {
    assert_eq!(ALLOWED_ASPECT_RATIOS.len(), 7);
    assert!(ALLOWED_ASPECT_RATIOS.contains(&"auto"));
    assert!(ALLOWED_ASPECT_RATIOS.contains(&"21:9"));
    assert!(!ALLOWED_ASPECT_RATIOS.contains(&"2:1"));
}

#[rstest]
#[case::model_path("fal-ai/kling-video", "fal-ai/kling-video")]
#[case::longer_model_path("fal-ai/kling-video/extra", "fal-ai/kling-video")]
#[case::one_segment("single", "single")]
#[case::empty("", "")]
#[case::workflow("workflows/fal-ai/name", "workflows/fal-ai/name")]
#[case::comfy("comfy/fal-ai/name", "comfy/fal-ai/name")]
#[case::workflow_extra("workflows/fal-ai/name/extra", "workflows/fal-ai/name")]
#[case::namespace_second("fal-ai/workflows/name", "fal-ai/workflows")]
fn queue_base_keeps_two_segments_or_three_for_namespaces(
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(queue_request_base_path(model), expected);
}

#[rstest]
#[case::plain("/fal-ai/model/requests/req-1/status", Some("fal-ai/model"))]
#[case::proxy_prefix("/v1/proxy/fal-ai/model/requests/req-1/status", Some("fal-ai/model"))]
#[case::workflow(
    "/workflows/fal-ai/name/requests/req-1/status",
    Some("workflows/fal-ai/name")
)]
#[case::workflow_proxy(
    "/proxy/workflows/fal-ai/name/requests/req-1/status",
    Some("workflows/fal-ai/name")
)]
#[case::no_requests("/fal-ai/model/req-1/status", None)]
#[case::single_segment("/fal-ai/requests/req-1", None)]
#[case::empty_model("/requests/req-1", None)]
#[case::requests_last("/fal-ai/model/requests", Some("fal-ai/model"))]
fn model_paths_come_from_the_first_requests_segment(
    #[case] path: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(model_path_from_request_url(path).as_deref(), expected);
}

#[rstest]
#[case::plain("/fal-ai/model/requests/req-1/status", Some("req-1"))]
#[case::proxy("/v1/fal-ai/model/requests/req-1", Some("req-1"))]
#[case::missing("/fal-ai/model/status", None)]
#[case::requests_last("/fal-ai/model/requests", None)]
#[case::first_requests_wins("/a/requests/one/requests/two", Some("one"))]
#[case::empty_segment_before_the_id("/fal-ai/model/requests//req-1", Some("req-1"))]
fn request_ids_follow_the_first_requests_segment(
    #[case] path: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(request_id_from_request_url(path).as_deref(), expected);
}

#[test]
fn complete_url_prefers_a_non_empty_explicit_base() {
    assert_eq!(
        complete_url(Some("https://explicit.fal.run/")),
        "https://explicit.fal.run"
    );
    assert_eq!(
        complete_url(Some("https://explicit.fal.run//")),
        "https://explicit.fal.run"
    );
    assert_eq!(complete_url(None), DEFAULT_QUEUE_BASE);
    assert_eq!(complete_url(Some("")), DEFAULT_QUEUE_BASE);
}

#[rstest]
#[case::explicit_wins(
    Some("explicit-key"),
    Some("params-key"),
    Some("environment-key"),
    Some("explicit-key")
)]
#[case::params_second(None, Some("params-key"), Some("environment-key"), Some("params-key"))]
#[case::environment_last(None, None, Some("environment-key"), Some("environment-key"))]
#[case::empty_explicit_falls_through(Some(""), Some("params-key"), None, Some("params-key"))]
#[case::missing(None, None, None, None)]
fn authorization_uses_the_first_non_empty_key(
    #[case] explicit: Option<&str>,
    #[case] param_key: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let settings = FalVideoSettings {
        api_key: environment.map(SecretString::from),
    };
    let headers = VideoHeaders::from([
        ("x-trace".to_owned(), "t-1".to_owned()),
        ("Authorization".to_owned(), "old".to_owned()),
    ]);
    let explicit_secret = explicit.map(SecretString::from);
    let param_secret = param_key.map(SecretString::from);
    match settings.authorize(&headers, explicit_secret.as_ref(), param_secret.as_ref()) {
        Ok(authorized) => {
            let expected_header = expected.map(|key| format!("Key {key}"));
            assert_eq!(
                authorized.get("Authorization").map(String::as_str),
                expected_header.as_deref()
            );
            assert_eq!(
                authorized.get("Content-Type").map(String::as_str),
                Some("application/json")
            );
            assert_eq!(authorized.get("x-trace").map(String::as_str), Some("t-1"));
        }
        Err(error) => {
            assert!(expected.is_none());
            assert!(matches!(error, FalVideoError::MissingApiKey));
        }
    }
}

#[test]
fn supported_params_are_the_seven_declared_options() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "model",
            "prompt",
            "input_reference",
            "seconds",
            "size",
            "user",
            "extra_headers"
        ]
    );
}

#[rstest]
#[case::h3_list(H3_MODEL, "https://a.test/i.png", "reference_image_urls", true)]
#[case::seedance_string(DEFAULT_MODEL, "https://a.test/i.png", "image_url", false)]
fn a_text_reference_maps_to_the_profile_key(
    #[case] model: &str,
    #[case] reference: &str,
    #[case] key: &str,
    #[case] as_list: bool,
) {
    let mapped = map_openai_params(
        &params(&[("input_reference", text(reference))]),
        model,
        false,
    )
    .expect("a text reference maps");
    let expected = if as_list {
        Json::List(vec![text(reference)])
    } else {
        text(reference)
    };
    assert_eq!(mapped.get(key), Some(&expected));
    assert_eq!(mapped.len(), 1);
}

#[rstest]
#[case::null(Json::Null)]
#[case::list(Json::List(vec![text("https://a.test/i.png")]))]
#[case::number(number("7"))]
#[case::boolean(Json::Bool(false))]
#[case::object(Json::Object(IndexMap::new()))]
fn a_present_non_text_reference_fails(#[case] reference: Json) {
    let mapped = map_openai_params(&params(&[("input_reference", reference)]), H3_MODEL, false);
    assert!(matches!(mapped, Err(FalVideoError::ReferenceNotText)));
}

#[test]
fn an_absent_reference_maps_nothing() {
    let mapped =
        map_openai_params(&VideoParams::new(), H3_MODEL, false).expect("a missing reference maps");
    assert!(mapped.is_empty());
}

#[rstest]
#[case::h3_auto(H3_MODEL, "auto", None)]
#[case::h3_integer(H3_MODEL, "4", Some(number("4")))]
#[case::h3_decimal(H3_MODEL, "4.9", Some(number("4")))]
#[case::seedance_auto(DEFAULT_MODEL, "auto", Some(text("auto")))]
#[case::seedance_integer(DEFAULT_MODEL, "4", Some(text("4")))]
#[case::seedance_decimal(DEFAULT_MODEL, "4.9", Some(text("4")))]
fn seconds_map_by_profile(
    #[case] model: &str,
    #[case] seconds: &str,
    #[case] expected: Option<Json>,
) {
    let mapped = map_openai_params(&params(&[("seconds", text(seconds))]), model, false)
        .expect("seconds map");
    assert_eq!(mapped.get("duration"), expected.as_ref());
}

#[rstest]
#[case::word(text("abc"))]
#[case::boolean(Json::Bool(true))]
#[case::null(Json::Null)]
#[case::list(Json::List(Vec::new()))]
fn non_numeric_seconds_fail(#[case] seconds: Json) {
    let mapped = map_openai_params(&params(&[("seconds", seconds)]), DEFAULT_MODEL, false);
    assert!(matches!(mapped, Err(FalVideoError::SecondsNotNumeric)));
}

#[test]
fn derived_fields_are_overlaid_by_unsupported_options() {
    let mapped = map_openai_params(
        &params(&[
            ("size", text("1024x768")),
            ("user", text("user-1")),
            ("resolution", text("720p")),
            ("characters", Json::List(Vec::new())),
        ]),
        DEFAULT_MODEL,
        true,
    )
    .expect("options map");
    assert_eq!(mapped.get("end_user_id"), Some(&text("user-1")));
    assert_eq!(mapped.get("resolution"), Some(&text("720p")));
    assert_eq!(mapped.get("characters"), Some(&Json::List(Vec::new())));
    assert!(!mapped.contains_key("size"));
    assert!(!mapped.contains_key("user"));
}

#[test]
fn supported_options_never_reach_the_upstream_body() {
    let mapped = map_openai_params(
        &params(&[
            ("model", text("other")),
            ("prompt", text("p")),
            ("extra_headers", Json::Object(IndexMap::new())),
        ]),
        DEFAULT_MODEL,
        false,
    )
    .expect("supported options map");
    assert!(mapped.is_empty());
}

#[test]
fn a_non_text_user_is_not_mapped() {
    let mapped =
        map_openai_params(&params(&[("user", number("5"))]), DEFAULT_MODEL, false).expect("user");
    assert!(mapped.is_empty());
}

#[test]
fn drop_params_does_not_change_the_mapping() {
    let options = params(&[("seconds", text("2.5")), ("resolution", text("1080p"))]);
    let dropped = map_openai_params(&options, DEFAULT_MODEL, true).expect("with drop_params");
    let kept = map_openai_params(&options, DEFAULT_MODEL, false).expect("without drop_params");
    assert_eq!(dropped, kept);
}

#[test]
fn create_request_overlays_options_on_the_prompt_except_model() {
    let options = params(&[
        ("model", text("ignored")),
        ("duration", text("2")),
        ("prompt", text("from options")),
    ]);
    let (data, url) = create_request(DEFAULT_MODEL, "draw", "https://queue.fal.run/", &options);
    assert_eq!(url, "https://queue.fal.run/fal-ai/kling-video");
    assert_eq!(data.get("prompt"), Some(&text("from options")));
    assert_eq!(data.get("duration"), Some(&text("2")));
    assert!(!data.contains_key("model"));
}

#[test]
fn a_queued_response_wraps_the_request_id_with_provider_and_model() {
    let reply = reply(r#"{"request_id": "req-1"}"#);
    let video = create_response(&CreateResponseInput {
        model: DEFAULT_MODEL,
        reply: &reply,
        custom_llm_provider: None,
        request_data: &VideoParams::new(),
        created_at: NOW,
    })
    .expect("a queued video");
    assert_eq!(video.object, VideoObjectType::Video);
    assert_eq!(video.status, "queued");
    assert_eq!(video.created_at, Some(NOW));
    assert_eq!(video.model.as_deref(), Some(DEFAULT_MODEL));
    assert_eq!(video.seconds, None);
    assert_eq!(video.size, None);
    assert_eq!(usage_text(&video, "duration_seconds"), None);
    assert_eq!(
        usage_text(&video, "video_resolution").as_deref(),
        Some("720p")
    );
    let target = decode_video_id(&video.id).expect("a model-bearing id");
    assert_eq!(target.request_id, "req-1");
    assert_eq!(target.model_id, DEFAULT_MODEL);
    assert_eq!(
        decode_video_id_with_provider(&video.id)
            .custom_llm_provider
            .as_deref(),
        Some(FAL_AI_PROVIDER)
    );
}

#[test]
fn a_queued_response_derives_seconds_size_and_usage_from_the_request() {
    let reply = reply(r#"{"request_id": "req-2"}"#);
    let request = params(&[("duration", text("2")), ("resolution", text("1080p"))]);
    let video = create_response(&CreateResponseInput {
        model: DEFAULT_MODEL,
        reply: &reply,
        custom_llm_provider: None,
        request_data: &request,
        created_at: NOW,
    })
    .expect("a queued video");
    assert_eq!(
        video.seconds.as_ref().and_then(|seconds| seconds.as_str()),
        Some("2")
    );
    assert_eq!(video.size.as_deref(), Some("1080p"));
    assert_eq!(
        usage_text(&video, "duration_seconds").as_deref(),
        Some("2.0")
    );
    assert_eq!(
        usage_text(&video, "video_resolution").as_deref(),
        Some("1080p")
    );
}

#[test]
fn an_integer_h3_duration_reads_back_as_seconds_and_a_float_usage() {
    let reply = reply(r#"{"request_id": "req-3"}"#);
    let request = params(&[("duration", number("4"))]);
    let video = create_response(&CreateResponseInput {
        model: H3_MODEL,
        reply: &reply,
        custom_llm_provider: None,
        request_data: &request,
        created_at: NOW,
    })
    .expect("a queued video");
    assert_eq!(
        video.seconds.as_ref().and_then(|seconds| seconds.as_str()),
        Some("4")
    );
    assert_eq!(
        usage_text(&video, "duration_seconds").as_deref(),
        Some("4.0")
    );
    assert_eq!(
        usage_text(&video, "video_resolution").as_deref(),
        Some("2K")
    );
}

#[test]
fn an_auto_duration_leaves_duration_usage_out() {
    let reply = reply(r#"{"request_id": "req-4"}"#);
    let request = params(&[("duration", text("auto"))]);
    let video = create_response(&CreateResponseInput {
        model: H3_MODEL,
        reply: &reply,
        custom_llm_provider: None,
        request_data: &request,
        created_at: NOW,
    })
    .expect("a queued video");
    assert_eq!(video.seconds, None);
    assert_eq!(usage_text(&video, "duration_seconds"), None);
    assert_eq!(
        usage_text(&video, "video_resolution").as_deref(),
        Some("2K")
    );
}

#[test]
fn an_infinite_duration_leaves_duration_usage_out() {
    let reply = reply(r#"{"request_id": "req-6"}"#);
    let request = params(&[("duration", text("inf"))]);
    let video = create_response(&CreateResponseInput {
        model: H3_MODEL,
        reply: &reply,
        custom_llm_provider: None,
        request_data: &request,
        created_at: NOW,
    })
    .expect("a queued video");
    assert_eq!(video.seconds, None);
    assert_eq!(usage_text(&video, "duration_seconds"), None);
    assert_eq!(
        usage_text(&video, "video_resolution").as_deref(),
        Some("2K")
    );
}

#[test]
fn a_non_text_resolution_falls_back_to_the_default_resolution() {
    let reply = reply(r#"{"request_id": "req-5"}"#);
    let request = params(&[("duration", text("2")), ("resolution", number("7"))]);
    let video = create_response(&CreateResponseInput {
        model: DEFAULT_MODEL,
        reply: &reply,
        custom_llm_provider: None,
        request_data: &request,
        created_at: NOW,
    })
    .expect("a queued video");
    assert_eq!(video.size, None);
    assert_eq!(
        usage_text(&video, "video_resolution").as_deref(),
        Some("720p")
    );
}

#[test]
fn a_missing_or_non_text_request_id_makes_an_empty_id() {
    let reply = reply(r#"{"request_id": 7}"#);
    let video = create_response(&CreateResponseInput {
        model: DEFAULT_MODEL,
        reply: &reply,
        custom_llm_provider: None,
        request_data: &VideoParams::new(),
        created_at: NOW,
    })
    .expect("a queued video");
    assert_eq!(video.id, "");
}

#[rstest]
#[case::default(None, "fal_ai")]
#[case::empty(Some(""), "fal_ai")]
#[case::custom(Some("fal_ai_custom"), "fal_ai_custom")]
fn the_provider_rides_on_the_encoded_id(#[case] provider: Option<&str>, #[case] expected: &str) {
    let reply = reply(r#"{"request_id": "req-6"}"#);
    let video = create_response(&CreateResponseInput {
        model: "workflows/fal-ai/name",
        reply: &reply,
        custom_llm_provider: provider,
        request_data: &VideoParams::new(),
        created_at: NOW,
    })
    .expect("a queued video");
    let decoded = decode_video_id_with_provider(&video.id);
    assert_eq!(decoded.custom_llm_provider.as_deref(), Some(expected));
    assert_eq!(decoded.model_id.as_deref(), Some("workflows/fal-ai/name"));
    assert_eq!(decoded.video_id, "req-6");
}

#[test]
fn job_urls_keep_the_queue_namespace_and_encode_the_request_id() {
    let video_id =
        encode_video_id_with_provider("req 1/..", "fal_ai", Some("workflows/fal-ai/name"));
    let urls = job_urls(&video_id, "https://queue.fal.run/").expect("job urls");
    assert_eq!(
        urls.status,
        "https://queue.fal.run/workflows/fal-ai/name/requests/req%201%2F../status"
    );
    assert_eq!(
        urls.content,
        "https://queue.fal.run/workflows/fal-ai/name/requests/req%201%2F.."
    );
}

#[test]
fn a_two_segment_model_drops_the_third_segment_in_the_queue_url() {
    let video_id =
        encode_video_id_with_provider("req-1", "fal_ai", Some("fal-ai/kling-video/extra"));
    let urls = job_urls(&video_id, "https://queue.fal.run").expect("job urls");
    assert_eq!(
        urls.status,
        "https://queue.fal.run/fal-ai/kling-video/requests/req-1/status"
    );
    assert_eq!(
        urls.content,
        "https://queue.fal.run/fal-ai/kling-video/requests/req-1"
    );
}

#[test]
fn a_video_id_without_a_model_cannot_make_a_url() {
    assert!(matches!(
        job_urls("plain-id", "https://queue.fal.run"),
        Err(FalVideoError::VideoIdWithoutModel)
    ));
    assert!(matches!(
        job_urls("", "https://queue.fal.run"),
        Err(FalVideoError::VideoIdWithoutModel)
    ));
}

#[test]
fn a_dot_request_id_is_refused_before_the_url_is_built() {
    let video_id = encode_video_id_with_provider("..", "fal_ai", Some("fal-ai/kling-video"));
    assert!(matches!(
        job_urls(&video_id, "https://queue.fal.run"),
        Err(FalVideoError::PathSegment(HttpError::DotSegment { .. }))
    ));
}

#[test]
fn decoding_a_video_id_returns_the_request_and_model() {
    let video_id = encode_video_id_with_provider("req-9", "fal_ai", Some("workflows/fal-ai/name"));
    let target = decode_video_id(&video_id).expect("a model-bearing id");
    assert_eq!(target.request_id, "req-9");
    assert_eq!(target.model_id, "workflows/fal-ai/name");
}

#[test]
fn a_full_reply_keeps_every_typed_field() {
    let body = r#"{"status": "COMPLETED", "request_id": "req-1", "video": {"url": "https://a.test/v.mp4", "content_type": "video/mp4"}, "detail": [{"loc": ["body", "prompt"], "msg": "is required"}, {"loc": "seconds", "msg": "bad"}], "error": "boom"}"#;
    let reply = reply(body);
    assert_eq!(reply.status.as_deref(), Some("COMPLETED"));
    assert_eq!(reply.request_id.as_deref(), Some("req-1"));
    assert_eq!(
        reply.video,
        Some(FalVideoResult {
            url: Some("https://a.test/v.mp4".to_owned())
        })
    );
    assert_eq!(reply.error.as_deref(), Some("boom"));
    assert_eq!(
        reply.detail,
        Some(FalVideoDetail::Items(vec![
            FalVideoDetailItem {
                msg: Some("is required".to_owned()),
                loc: Some(FalVideoLocation::Parts(vec![
                    "body".to_owned(),
                    "prompt".to_owned()
                ])),
            },
            FalVideoDetailItem {
                msg: Some("bad".to_owned()),
                loc: Some(FalVideoLocation::Text("seconds".to_owned())),
            },
        ]))
    );
}

#[test]
fn an_empty_object_keeps_no_field() {
    assert_eq!(reply("{}"), FalVideoReply::default());
}

#[test]
fn only_text_fields_are_kept() {
    let reply =
        reply(r#"{"status": 5, "request_id": null, "video": "v", "detail": 7, "error": ["e"]}"#);
    assert_eq!(reply.status, None);
    assert_eq!(reply.request_id, None);
    assert_eq!(reply.video, None);
    assert_eq!(reply.detail, None);
    assert_eq!(reply.error, None);
}

#[test]
fn a_video_url_is_kept_only_when_it_is_a_non_empty_string() {
    assert_eq!(
        reply(r#"{"video": {"url": ""}}"#).video,
        Some(FalVideoResult { url: None })
    );
    assert_eq!(
        reply(r#"{"video": {"url": 5}}"#).video,
        Some(FalVideoResult { url: None })
    );
    assert_eq!(
        reply(r#"{"video": {}}"#).video,
        Some(FalVideoResult { url: None })
    );
}

#[test]
fn detail_items_skip_non_objects_and_keep_typed_locations() {
    let reply =
        reply(r#"{"detail": [{"loc": ["a", 5, "b"], "msg": 7}, 5, {"loc": []}, {"loc": ""}]}"#);
    let Some(FalVideoDetail::Items(items)) = reply.detail else {
        panic!("a detail list");
    };
    assert_eq!(items.len(), 3);
    assert_eq!(
        items[0],
        FalVideoDetailItem {
            msg: None,
            loc: Some(FalVideoLocation::Parts(vec![
                "a".to_owned(),
                "b".to_owned()
            ])),
        }
    );
    assert_eq!(
        items[1],
        FalVideoDetailItem {
            msg: None,
            loc: None,
        }
    );
    assert_eq!(
        items[2],
        FalVideoDetailItem {
            msg: None,
            loc: None,
        }
    );
}

#[test]
fn a_text_detail_stays_whole() {
    assert_eq!(
        reply(r#"{"detail": "bad request"}"#).detail,
        Some(FalVideoDetail::Text("bad request".to_owned()))
    );
}

#[test]
fn a_non_object_root_is_refused() {
    let headers = VideoHeaders::from([("x-request-id".to_owned(), "r-1".to_owned())]);
    let result = decode_fal_video_reply(413, &headers, b"[]");
    let Err(FalVideoError::ReplyNotAnObject {
        status,
        headers: kept,
    }) = result
    else {
        panic!("a non-object root is refused");
    };
    assert_eq!(status, 413);
    assert_eq!(kept, headers);
}

#[test]
fn a_body_that_is_not_json_is_refused() {
    let headers = VideoHeaders::from([("x-request-id".to_owned(), "r-1".to_owned())]);
    let result = decode_fal_video_reply(502, &headers, b"not json");
    let Err(FalVideoError::ReplyDecode {
        status,
        headers: kept,
        ..
    }) = result
    else {
        panic!("a body that is not JSON is refused");
    };
    assert_eq!(status, 502);
    assert_eq!(kept, headers);
}

#[test]
fn a_body_over_the_limit_is_refused_before_parsing() {
    let body = vec![b' '; MAX_FAL_VIDEO_REPLY_BYTES.saturating_add(1)];
    let headers = VideoHeaders::from([("x-request-id".to_owned(), "r-1".to_owned())]);
    let result = decode_fal_video_reply(413, &headers, &body);
    let Err(FalVideoError::ReplyTooLarge {
        status,
        headers: kept,
        limit,
    }) = result
    else {
        panic!("a body over the limit is refused");
    };
    assert_eq!(status, 413);
    assert_eq!(kept, headers);
    assert_eq!(limit, MAX_FAL_VIDEO_REPLY_BYTES);
}

#[test]
fn a_body_exactly_at_the_limit_is_parsed() {
    let body = vec![b' '; MAX_FAL_VIDEO_REPLY_BYTES];
    let result = decode_fal_video_reply(200, &VideoHeaders::new(), &body);
    assert!(matches!(result, Err(FalVideoError::ReplyDecode { .. })));
}

#[test]
fn the_error_class_keeps_the_status_message_and_headers() {
    let headers = VideoHeaders::from([("x-request-id".to_owned(), "r-1".to_owned())]);
    let error = get_error_class("bad request", 400, &headers);
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, "bad request");
    assert_eq!(error.headers, headers);
}

#[rstest]
fn replays_the_fal_video_reply_corpus(#[files("fuzz/corpus/fal_video_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("a corpus file");
    drop(decode_fal_video_reply(200, &VideoHeaders::new(), &bytes));
}
