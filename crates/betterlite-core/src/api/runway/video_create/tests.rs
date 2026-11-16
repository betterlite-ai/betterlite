#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::openai_video_identifiers::ids::decode_video_id_with_provider;
use betterlite_openai::videos::reply::{VideoObject, VideoObjectType};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Number, json};

use super::VideoParams;
use super::credentials::{VideoCredentials, validate_environment};
use super::error::{RunwayMLError, RunwayVideoError, get_error_class};
use super::params::map_openai_params;
use super::reply::{create_response, map_status, parse_timestamp};
use super::request::create_request;
use super::usage::{estimated_cost_usd, progress_percent, ratio_to_resolution};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn lexeme(value: &str) -> Json {
    Json::Number(Number::from_str(value).unwrap())
}

fn params(pairs: &[(&str, Json)]) -> VideoParams {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn credentials(
    explicit: Option<&str>,
    typed: Option<&str>,
    global: Option<&str>,
    environment: &[(&str, &str)],
) -> VideoCredentials {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let mut found = VideoCredentials::from_environment(&lookup);
    found.api_key = explicit.map(|value| SecretString::from(value.to_owned()));
    found.params_api_key = typed.map(|value| SecretString::from(value.to_owned()));
    found.global_api_key = global.map(|value| SecretString::from(value.to_owned()));
    found
}

fn authorization(found: &VideoCredentials) -> Option<String> {
    validate_environment(&Headers::new(), found, "2024-11-06")
        .ok()
        .and_then(|headers| headers.get("Authorization").cloned())
}

fn created(body: &str, provider: Option<&str>, request: &VideoParams) -> VideoObject {
    create_response("gen4_turbo", body.as_bytes(), provider, request).unwrap()
}

#[test]
fn endpoint_priority_reads_present_non_null_inputs() {
    let image = create_request("m", "p", None, &params(&[("promptImage", text("i"))]));
    assert_eq!(
        image.url, "https://api.dev.runwayml.com/v1/image_to_video",
        "a non-null promptImage selects image_to_video"
    );
    let video = create_request("m", "p", None, &params(&[("videoUri", text("v"))]));
    assert_eq!(
        video.url, "https://api.dev.runwayml.com/v1/video_to_video",
        "a non-null videoUri selects video_to_video"
    );
    let empty_video = params(&[("promptVideo", text("")), ("promptImage", text("i"))]);
    assert_eq!(
        create_request("m", "p", None, &empty_video).url,
        "https://api.dev.runwayml.com/v1/video_to_video",
        "an empty promptVideo is still present, so it wins over the image"
    );
    let null_video = params(&[("promptVideo", Json::Null), ("promptImage", text("i"))]);
    assert_eq!(
        create_request("m", "p", None, &null_video).url,
        "https://api.dev.runwayml.com/v1/image_to_video",
        "a null promptVideo falls through to the image"
    );
    assert_eq!(
        create_request("m", "p", None, &VideoParams::new()).url,
        "https://api.dev.runwayml.com/v1/text_to_video"
    );
}

#[test]
fn prompt_image_survives_only_the_image_endpoint() {
    let image = create_request("m", "p", None, &params(&[("promptImage", text("i"))]));
    assert_eq!(image.body.get("promptImage"), Some(&text("i")));
    let video = params(&[("promptImage", text("i")), ("videoUri", text("v"))]);
    assert_eq!(
        create_request("m", "p", None, &video)
            .body
            .get("promptImage"),
        None
    );
    let text_only = create_request("m", "p", None, &params(&[("promptImage", Json::Null)]));
    assert_eq!(text_only.body.get("promptImage"), None);
}

#[test]
fn request_body_puts_the_constructed_fields_first_and_lets_params_override_them() {
    let given = params(&[("promptText", text("other")), ("seed", number(7))]);
    let form = create_request("gen4_turbo", "a cat", None, &given);
    assert_eq!(form.body.get("model"), Some(&text("gen4_turbo")));
    assert_eq!(form.body.get("promptText"), Some(&text("other")));
    assert_eq!(form.body.get("seed"), Some(&number(7)));
    assert_eq!(form.body.keys().next().map(String::as_str), Some("model"));
}

#[test]
fn openai_parameters_map_to_provider_fields() {
    let given = params(&[
        ("input_reference", Json::Null),
        ("size", text("1280x720x2")),
        ("seconds", text("2.9")),
    ]);
    let mapped = map_openai_params(&given);
    assert_eq!(mapped.get("promptImage"), Some(&Json::Null));
    assert_eq!(mapped.get("ratio"), Some(&text("1280:720:2")));
    assert_eq!(mapped.get("duration"), Some(&number(2)));
}

#[test]
fn advertised_parameters_do_not_pass_through_but_others_win_last() {
    let given = params(&[
        ("model", text("m")),
        ("prompt", text("p")),
        ("user", text("u")),
        ("extra_headers", Json::Object(VideoParams::new())),
        ("size", text("64x64")),
        ("ratio", text("1:1")),
        ("duration", lexeme("9")),
        ("seed", number(3)),
    ]);
    let mapped = map_openai_params(&given);
    assert_eq!(mapped.get("model"), None);
    assert_eq!(mapped.get("prompt"), None);
    assert_eq!(mapped.get("user"), None);
    assert_eq!(mapped.get("extra_headers"), None);
    assert_eq!(mapped.get("ratio"), Some(&text("1:1")));
    assert_eq!(mapped.get("duration"), Some(&number(9)));
    assert_eq!(mapped.get("seed"), Some(&number(3)));
}

#[rstest]
#[case::text_fraction(text("2.9"), Some(number(2)))]
#[case::text_negative_fraction(text("-2.9"), Some(number(-2)))]
#[case::text_whitespace(text(" 3.7 "), Some(number(3)))]
#[case::text_invalid(text("soon"), None)]
#[case::text_nan(text("NaN"), None)]
#[case::text_infinite(text("inf"), None)]
#[case::number_fraction(lexeme("2.9"), Some(number(2)))]
#[case::number_precise_fraction(lexeme("2.9999999999999999999"), Some(number(3)))]
#[case::number_large_integer(lexeme("9007199254740993"), Some(lexeme("9007199254740993")))]
#[case::number_beyond_decimal(lexeme("1e40"), None)]
#[case::bool_true(Json::Bool(true), Some(number(1)))]
#[case::bool_false(Json::Bool(false), Some(number(0)))]
#[case::null(Json::Null, None)]
#[case::list(Json::List(Vec::new()), None)]
#[case::object(Json::Object(VideoParams::new()), None)]
fn seconds_truncate_toward_zero_and_failed_conversions_omit_duration(
    #[case] seconds: Json,
    #[case] expected: Option<Json>,
) {
    let mapped = map_openai_params(&params(&[("seconds", seconds)]));
    assert_eq!(mapped.get("duration"), expected.as_ref());
}

#[test]
fn null_base_defaults_and_trailing_slashes_are_trimmed() {
    let defaulted = create_request("m", "p", None, &VideoParams::new());
    assert_eq!(
        defaulted.url,
        "https://api.dev.runwayml.com/v1/text_to_video"
    );
    let empty = create_request("m", "p", Some(""), &VideoParams::new());
    assert_eq!(empty.url, "/text_to_video");
    let slashed = create_request(
        "m",
        "p",
        Some("https://runway.test/v1///"),
        &VideoParams::new(),
    );
    assert_eq!(slashed.url, "https://runway.test/v1/text_to_video");
}

#[rstest]
#[case::explicit(Some("explicit"), Some("typed"), Some("global"), &[("RUNWAYML_API_SECRET", "s"), ("RUNWAYML_API_KEY", "k")], Some("Bearer explicit"))]
#[case::empty_explicit(Some(""), Some("typed"), Some("global"), &[("RUNWAYML_API_SECRET", "s")], Some("Bearer typed"))]
#[case::typed(None, Some("typed"), Some("global"), &[("RUNWAYML_API_SECRET", "s")], Some("Bearer typed"))]
#[case::empty_typed(None, Some(""), Some("global"), &[], Some("Bearer global"))]
#[case::global(None, None, Some("global"), &[], Some("Bearer global"))]
#[case::secret(None, None, None, &[("RUNWAYML_API_SECRET", "s"), ("RUNWAYML_API_KEY", "k")], Some("Bearer s"))]
#[case::empty_secret(None, None, None, &[("RUNWAYML_API_SECRET", ""), ("RUNWAYML_API_KEY", "k")], Some("Bearer k"))]
#[case::empty_last_key(None, None, None, &[("RUNWAYML_API_KEY", "")], Some("Bearer "))]
#[case::nothing(None, None, None, &[], None)]
fn credentials_prefer_the_explicit_key_then_fall_through_to_the_last_environment_value(
    #[case] explicit: Option<&str>,
    #[case] typed: Option<&str>,
    #[case] global: Option<&str>,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        authorization(&credentials(explicit, typed, global, environment)),
        expected.map(str::to_owned)
    );
}

#[test]
fn a_missing_key_from_every_source_is_refused() {
    let found = credentials(None, None, None, &[]);
    let failed = validate_environment(&Headers::new(), &found, "2024-11-06").unwrap_err();
    assert!(matches!(failed, RunwayVideoError::MissingCredentials));
    assert!(failed.to_string().contains("RUNWAYML_API_SECRET"));
}

#[test]
fn authentication_adds_the_version_and_json_content_type() {
    let found = credentials(Some("k"), None, None, &[]);
    let headers = validate_environment(&Headers::new(), &found, "2025-03-03").unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer k")
    );
    assert_eq!(
        headers.get("X-Runway-Version").map(String::as_str),
        Some("2025-03-03")
    );
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[rstest]
#[case::succeeded("SUCCEEDED", "completed")]
#[case::succeeded_lower("succeeded", "completed")]
#[case::running("RUNNING", "in_progress")]
#[case::failed("FAILED", "failed")]
#[case::cancelled("CANCELLED", "failed")]
#[case::throttled("THROTTLED", "queued")]
#[case::pending("PENDING", "queued")]
#[case::unknown("UNKNOWN_STATE", "queued")]
fn status_names_map_case_insensitively(#[case] status: &str, #[case] expected: &str) {
    assert_eq!(map_status(Some(status)), expected);
}

#[test]
fn missing_status_is_queued() {
    assert_eq!(map_status(None), "queued");
}

#[test]
fn timestamps_decode_offsets_and_zero_everything_else() {
    assert_eq!(
        parse_timestamp(Some(&text("2025-11-11T21:48:50.448Z"))),
        1_762_897_730
    );
    assert_eq!(
        parse_timestamp(Some(&text("2025-11-11T23:48:50+02:00"))),
        1_762_897_730
    );
    assert_eq!(
        parse_timestamp(Some(&text("2025-11-11T21:48:50"))),
        1_762_897_730
    );
    assert_eq!(parse_timestamp(Some(&text("1969-12-31T23:59:59.5Z"))), 0);
    assert_eq!(parse_timestamp(Some(&text("yesterday"))), 0);
    assert_eq!(parse_timestamp(Some(&text(""))), 0);
    assert_eq!(parse_timestamp(Some(&Json::Null)), 0);
    assert_eq!(parse_timestamp(Some(&number(1_762_897_730))), 0);
    assert_eq!(parse_timestamp(None), 0);
}

#[test]
fn creation_keeps_identity_completion_presence_and_progress_absence() {
    let body = r#"{"id":"task_1","status":"PENDING","createdAt":"2025-11-11T21:48:50Z"}"#;
    let video = created(body, None, &VideoParams::new());
    assert_eq!(video.id, "task_1");
    assert_eq!(video.object, VideoObjectType::Video);
    assert_eq!(video.status, "queued");
    assert_eq!(video.created_at, Some(1_762_897_730));
    assert_eq!(video.completed_at, None);
    assert_eq!(video.progress, None);
    assert_eq!(video.expires_at, None);
    assert_eq!(video.remixed_from_video_id, None);
    let completed = created(
        r#"{"id":"task_1","status":"SUCCEEDED","completedAt":null,"failure":null}"#,
        None,
        &VideoParams::new(),
    );
    assert_eq!(completed.completed_at, Some(0));
    assert_eq!(completed.status, "completed");
}

#[test]
fn a_present_failure_field_creates_an_error_without_forcing_the_status() {
    let absent = created(
        r#"{"id":"t","status":"SUCCEEDED"}"#,
        None,
        &VideoParams::new(),
    );
    assert_eq!(absent.error, None);
    let blank = created(
        r#"{"id":"t","status":"SUCCEEDED","failureCode":""}"#,
        None,
        &VideoParams::new(),
    );
    let error = blank.error.unwrap();
    assert_eq!(error.get("code"), Some(&json!("")));
    assert_eq!(
        error.get("message"),
        Some(&json!("Video generation failed"))
    );
    assert_eq!(blank.status, "completed");
    let nulls = created(
        r#"{"id":"t","failureCode":null,"failure":null}"#,
        None,
        &VideoParams::new(),
    );
    let error = nulls.error.unwrap();
    assert_eq!(error.get("code"), Some(&json!(null)));
    assert_eq!(error.get("message"), Some(&json!(null)));
}

#[test]
fn provider_tagging_leaves_empty_ids_alone() {
    let body = r#"{"id":"task_1","status":"PENDING"}"#;
    let tagged = created(body, Some("runwayml"), &VideoParams::new());
    let decoded = decode_video_id_with_provider(&tagged.id);
    assert_eq!(decoded.custom_llm_provider.as_deref(), Some("runwayml"));
    assert_eq!(decoded.video_id, "task_1");
    assert_eq!(decoded.model_id.as_deref(), Some("gen4_turbo"));
    let untagged = created(body, None, &VideoParams::new());
    assert_eq!(untagged.id, "task_1");
    let empty_provider = created(body, Some(""), &VideoParams::new());
    assert_eq!(empty_provider.id, "task_1");
    let empty_id = created(
        r#"{"id":"","status":"PENDING"}"#,
        Some("runwayml"),
        &VideoParams::new(),
    );
    assert_eq!(empty_id.id, "");
}

#[test]
fn request_fields_reach_the_reply_and_colons_become_x_in_the_size() {
    let given = params(&[
        ("input_reference", text("image")),
        ("seconds", text("4.9")),
        ("size", text("1280x720")),
    ]);
    let form = create_request("gen4_turbo", "a cat", None, &map_openai_params(&given));
    let video = created(r#"{"id":"t","status":"PENDING"}"#, None, &form.body);
    assert_eq!(video.model.as_deref(), Some("gen4_turbo"));
    assert_eq!(video.size.as_deref(), Some("1280x720"));
    assert_eq!(video.seconds, Some(json!("4")));
}

#[rstest]
#[case::below_480p("800:749", Some("480p"))]
#[case::exactly_600000("800:750", Some("720p"))]
#[case::below_720p("1499:1000", Some("720p"))]
#[case::exactly_1500000("1500:1000", Some("1080p"))]
#[case::below_1080p("1999:2000", Some("1080p"))]
#[case::exactly_4000000("2000:2000", Some("4k"))]
#[case::zero("0:0", Some("480p"))]
#[case::leading_zeros("0800:0750", Some("720p"))]
#[case::not_a_ratio("1280", None)]
#[case::empty_side(":720", None)]
#[case::non_digits("1280x720", None)]
#[case::second_ratio("1280:720:2", None)]
#[case::signed("-800:720", None)]
#[case::unicode_digits("١٢٨٠:720", None)]
#[case::huge("99999999999999999999999999999999999999999:2", Some("4k"))]
fn ratio_to_resolution_follows_the_area_tiers(#[case] ratio: &str, #[case] expected: Option<&str>) {
    assert_eq!(ratio_to_resolution(Some(&text(ratio))), expected);
}

#[test]
fn resolution_reads_only_text_ratios() {
    assert_eq!(ratio_to_resolution(None), None);
    assert_eq!(ratio_to_resolution(Some(&number(1_280))), None);
    assert_eq!(ratio_to_resolution(Some(&Json::Null)), None);
}

#[rstest]
#[case::ten(lexeme("10"), Some(json!(0.1)))]
#[case::fraction(lexeme("0.5"), Some(json!(0.005)))]
#[case::two_and_a_half(lexeme("2.5"), Some(json!(0.025)))]
#[case::hundred(lexeme("100"), Some(json!(1.0)))]
#[case::zero(lexeme("0"), Some(json!(0.0)))]
#[case::bool_true(Json::Bool(true), Some(json!(0.01)))]
#[case::bool_false(Json::Bool(false), Some(json!(0.0)))]
#[case::text(Json::Text("10".to_owned()), None)]
#[case::null(Json::Null, None)]
#[case::list(Json::List(Vec::new()), None)]
#[case::beyond_decimal(lexeme("1e40"), None)]
fn credits_become_usd_at_a_cent_each(#[case] credits: Json, #[case] expected: Option<Json>) {
    assert_eq!(estimated_cost_usd(Some(&credits)), expected);
    assert_eq!(estimated_cost_usd(None), None);
}

#[test]
fn usage_carries_duration_resolution_and_reported_credits() {
    let given = params(&[("seconds", text("5")), ("size", text("1280x720"))]);
    let form = create_request("gen4_turbo", "a cat", None, &map_openai_params(&given));
    let body = r#"{"id":"t","status":"PENDING","estimatedCost":{"credits":10}}"#;
    let usage = created(body, None, &form.body).usage.unwrap();
    assert_eq!(usage.get("duration_seconds"), Some(&json!(5.0)));
    assert_eq!(usage.get("video_resolution"), Some(&json!("720p")));
    assert_eq!(usage.get("provider_reported_cost_usd"), Some(&json!(0.1)));
}

#[test]
fn an_unparseable_seconds_text_leaves_the_duration_out_of_usage() {
    let given = params(&[("duration", text("soon"))]);
    let body = r#"{"id":"t","status":"PENDING"}"#;
    let video = created(body, None, &given);
    assert_eq!(video.seconds, Some(json!("soon")));
    assert_eq!(video.usage.unwrap().get("duration_seconds"), None);
}

#[test]
fn a_non_scalar_duration_leaves_seconds_out() {
    let null = params(&[("duration", Json::Null)]);
    assert_eq!(created(r#"{"id":"t"}"#, None, &null).seconds, None);
    let list = params(&[("duration", Json::List(vec![number(5)]))]);
    assert_eq!(created(r#"{"id":"t"}"#, None, &list).seconds, None);
}

#[rstest]
#[case::zero(0.0, 0)]
#[case::small(0.005, 0)]
#[case::tie_up(0.015, 2)]
#[case::tie_down(0.025, 2)]
#[case::half(0.5, 50)]
#[case::float_tie_above(0.545, 54)]
#[case::float_tie_below(0.575, 58)]
#[case::whole(1.0, 100)]
#[case::over(1.5, 100)]
#[case::negative(-0.5, 0)]
fn progress_rounds_half_even_and_clamps(#[case] progress: f64, #[case] expected: i64) {
    assert_eq!(progress_percent(progress), expected);
}

#[test]
fn progress_clamps_non_finite_shares() {
    assert_eq!(progress_percent(f64::NAN), 0);
    assert_eq!(progress_percent(f64::INFINITY), 100);
}

#[test]
fn get_error_class_keeps_the_status_message_and_headers() {
    let headers = Headers::from([
        ("content-type".to_owned(), "application/json".to_owned()),
        ("x-request-id".to_owned(), "req-1".to_owned()),
    ]);
    let RunwayMLError {
        status_code,
        message,
        headers: raised,
    } = get_error_class("the task is gone", 404, &headers);
    assert_eq!(status_code, 404);
    assert_eq!(message, "the task is gone");
    assert_eq!(
        raised,
        BTreeMap::from([
            (
                "content-type".to_owned(),
                Json::Text("application/json".to_owned())
            ),
            ("x-request-id".to_owned(), Json::Text("req-1".to_owned())),
        ])
    );
}
