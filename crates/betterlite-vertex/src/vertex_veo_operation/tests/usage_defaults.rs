use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde_json::Number;
use serde_json::json;

use super::{number, object, request_data, settings, text};
use crate::vertex_veo_operation::error::VeoOperationError;
use crate::vertex_veo_operation::usage::video_usage;

#[test]
fn a_request_without_parameters_has_empty_usage() {
    let missing = video_usage(None, &settings(8)).expect("no request data converts");
    assert_eq!(serde_json::to_value(&missing).unwrap(), json!({}));
    let empty = video_usage(Some(&object([])), &settings(8)).expect("empty data converts");
    assert_eq!(serde_json::to_value(&empty).unwrap(), json!({}));
}

#[test]
fn a_null_parameters_block_is_the_empty_default() {
    let data = object([("parameters", Json::Null)]);
    let usage = video_usage(Some(&data), &settings(8)).expect("the parameters convert");
    assert_eq!(
        serde_json::to_value(&usage).unwrap(),
        json!({"duration_seconds": 8.0})
    );
}

#[test]
fn a_missing_parameters_key_still_carries_the_default_duration() {
    let data = object([("instances", Json::List(Vec::new()))]);
    let usage = video_usage(Some(&data), &settings(8)).expect("the parameters convert");
    assert_eq!(
        serde_json::to_value(&usage).unwrap(),
        json!({"duration_seconds": 8.0})
    );
}

#[test]
fn a_missing_duration_is_the_configured_default() {
    let parameters = object([("resolution", text("720p"))]);
    let usage =
        video_usage(Some(&request_data(parameters)), &settings(11)).expect("the usage converts");
    assert_eq!(
        serde_json::to_value(&usage).unwrap(),
        json!({"duration_seconds": 11.0, "video_resolution": "720p"})
    );
}

#[rstest]
#[case::null(Json::Null)]
#[case::zero(number(0))]
#[case::false_flag(Json::Bool(false))]
#[case::empty_text(text(""))]
fn a_falsy_duration_is_the_configured_default(#[case] duration: Json) {
    let parameters = object([("durationSeconds", duration)]);
    let usage =
        video_usage(Some(&request_data(parameters)), &settings(8)).expect("the usage converts");
    assert_eq!(
        serde_json::to_value(&usage).unwrap(),
        json!({"duration_seconds": 8.0})
    );
}

#[rstest]
#[case::integer(number(6), 6.0)]
#[case::negative(number(-3), -3.0)]
#[case::decimal(Json::Number(Number::from_f64(6.5).unwrap()), 6.5)]
#[case::decimal_text(text("6.5"), 6.5)]
#[case::padded_text(text(" 12 "), 12.0)]
fn a_readable_duration_is_a_float(#[case] duration: Json, #[case] seconds: f64) {
    let parameters = object([("durationSeconds", duration)]);
    let usage =
        video_usage(Some(&request_data(parameters)), &settings(8)).expect("the usage converts");
    assert_eq!(
        serde_json::to_value(&usage).unwrap(),
        json!({"duration_seconds": seconds})
    );
}

#[rstest]
#[case::true_flag(Json::Bool(true))]
#[case::words(text("six"))]
#[case::list(Json::List(vec![number(6)]))]
#[case::object(Json::Object(object([("seconds", number(6))])))]
fn an_unreadable_duration_is_omitted(#[case] duration: Json) {
    let parameters = object([("durationSeconds", duration)]);
    let usage =
        video_usage(Some(&request_data(parameters)), &settings(8)).expect("the usage converts");
    assert_eq!(serde_json::to_value(&usage).unwrap(), json!({}));
}

#[test]
fn empty_parameters_keep_the_default_duration() {
    let usage =
        video_usage(Some(&request_data(object([]))), &settings(8)).expect("the usage converts");
    assert_eq!(
        serde_json::to_value(&usage).unwrap(),
        json!({"duration_seconds": 8.0})
    );
}

#[rstest]
#[case::uppercase(text(" 1080P "), "1080p")]
#[case::number(number(1080), "1080")]
#[case::zero(number(0), "0")]
fn a_resolution_is_trimmed_and_lowercased(#[case] resolution: Json, #[case] expected: &str) {
    let parameters = object([("resolution", resolution)]);
    let usage =
        video_usage(Some(&request_data(parameters)), &settings(8)).expect("the usage converts");
    assert_eq!(
        serde_json::to_value(&usage).unwrap(),
        json!({"duration_seconds": 8.0, "video_resolution": expected})
    );
}

#[rstest]
#[case::null(Json::Null)]
#[case::empty(text(""))]
#[case::spaces(text("   "))]
fn an_absent_resolution_is_omitted(#[case] resolution: Json) {
    let parameters = object([("resolution", resolution)]);
    let usage =
        video_usage(Some(&request_data(parameters)), &settings(8)).expect("the usage converts");
    assert_eq!(
        serde_json::to_value(&usage).unwrap(),
        json!({"duration_seconds": 8.0})
    );
}

// str(res) on a bool or a container is Python text, not a resolution, so the field is omitted like an unreadable duration
#[rstest]
#[case::flag(Json::Bool(true))]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(Object::new()))]
fn a_resolution_that_is_not_text_or_a_number_is_omitted(#[case] resolution: Json) {
    let parameters = object([("resolution", resolution)]);
    let usage =
        video_usage(Some(&request_data(parameters)), &settings(8)).expect("the usage converts");
    assert!(usage.get("video_resolution").is_none());
}

#[rstest]
#[case::one(number(1), 1)]
#[case::many(number(4), 4)]
fn a_positive_sample_count_is_the_video_count(#[case] sample_count: Json, #[case] count: u64) {
    let parameters = object([("sampleCount", sample_count)]);
    let usage =
        video_usage(Some(&request_data(parameters)), &settings(8)).expect("the usage converts");
    assert_eq!(
        serde_json::to_value(&usage).unwrap(),
        json!({"duration_seconds": 8.0, "video_count": count})
    );
}

#[rstest]
#[case::zero(number(0))]
#[case::negative(number(-1))]
#[case::true_flag(Json::Bool(true))]
#[case::false_flag(Json::Bool(false))]
#[case::decimal(Json::Number(Number::from_f64(2.0).unwrap()))]
#[case::text(text("2"))]
#[case::null(Json::Null)]
fn a_sample_count_that_is_not_a_positive_integer_is_omitted(#[case] sample_count: Json) {
    let parameters = object([("sampleCount", sample_count)]);
    let usage =
        video_usage(Some(&request_data(parameters)), &settings(8)).expect("the usage converts");
    assert!(usage.get("video_count").is_none());
}

#[rstest]
#[case::text(text("x"))]
#[case::number(number(1))]
#[case::flag(Json::Bool(true))]
#[case::list(Json::List(Vec::new()))]
fn parameters_that_are_not_an_object_are_rejected(#[case] parameters: Json) {
    let data = object([("parameters", parameters)]);
    assert!(matches!(
        video_usage(Some(&data), &settings(8)),
        Err(VeoOperationError::ParametersShape)
    ));
}
