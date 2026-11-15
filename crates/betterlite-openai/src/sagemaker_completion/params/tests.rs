#![cfg(test)]

use rstest::rstest;
use serde_json::{Value, json};

use super::{Options, SUPPORTED_OPENAI_PARAMS, map_openai_params};

fn options(pairs: (&str, Value)) -> Options {
    Options::from([(pairs.0.to_owned(), pairs.1)])
}

#[test]
fn a_zero_temperature_is_rewritten_to_the_positive_floor() {
    let mut requested = options(("temperature", json!(0.0_f64)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("temperature"), Some(&json!(0.01_f64)));
}

#[test]
fn an_integer_zero_temperature_is_rewritten_to_the_positive_floor() {
    let mut requested = options(("temperature", json!(0_i32)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("temperature"), Some(&json!(0.01_f64)));
}

#[test]
fn a_positive_temperature_is_kept_as_given() {
    let mut requested = options(("temperature", json!(0.7_f64)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("temperature"), Some(&json!(0.7_f64)));
}

#[test]
fn the_allow_zero_temperature_flag_keeps_a_zero_temperature() {
    let mut requested = Options::from([
        (String::from("temperature"), json!(0_i32)),
        (String::from("aws_sagemaker_allow_zero_temp"), json!(true)),
    ]);
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("temperature"), Some(&json!(0_i32)));
}

#[test]
fn the_allow_zero_temperature_flag_is_removed_from_the_callers_options() {
    let mut requested = options(("aws_sagemaker_allow_zero_temp", json!(true)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert!(requested.is_empty());
    assert!(mapped.is_empty());
}

#[test]
fn n_writes_best_of_and_turns_sampling_on() {
    let mut requested = options(("n", json!(3_i32)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("best_of"), Some(&json!(3_i32)));
    assert_eq!(mapped.get("do_sample"), Some(&json!(true)));
}

#[test]
fn a_zero_max_tokens_becomes_one() {
    let mut requested = options(("max_tokens", json!(0_i32)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("max_new_tokens"), Some(&json!(1_i32)));
}

#[test]
fn a_positive_max_tokens_is_kept_as_given() {
    let mut requested = options(("max_tokens", json!(64_i32)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("max_new_tokens"), Some(&json!(64_i32)));
}

#[test]
fn max_tokens_after_max_completion_tokens_wins() {
    let mut requested = Options::from([
        (String::from("max_completion_tokens"), json!(70_i32)),
        (String::from("max_tokens"), json!(0_i32)),
    ]);
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("max_new_tokens"), Some(&json!(1_i32)));
}

#[test]
fn max_completion_tokens_after_max_tokens_wins() {
    let mut requested = Options::from([
        (String::from("max_tokens"), json!(0_i32)),
        (String::from("max_completion_tokens"), json!(70_i32)),
    ]);
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("max_new_tokens"), Some(&json!(70_i32)));
}

#[test]
fn stop_top_p_and_stream_pass_through_as_given() {
    let mut requested = Options::from([
        (String::from("stop"), json!(["END"])),
        (String::from("top_p"), json!(0.9_f64)),
        (String::from("stream"), json!(false)),
    ]);
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("stop"), Some(&json!(["END"])));
    assert_eq!(mapped.get("top_p"), Some(&json!(0.9_f64)));
    assert_eq!(mapped.get("stream"), Some(&json!(false)));
}

#[test]
fn an_option_outside_the_supported_list_is_not_mapped() {
    let mut requested = options(("frequency_penalty", json!(1_i32)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert!(mapped.is_empty());
}

#[test]
fn the_supported_options_keep_the_pinned_order() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "stream",
            "temperature",
            "max_tokens",
            "max_completion_tokens",
            "top_p",
            "stop",
            "n"
        ]
    );
}

#[rstest]
#[case::falsy_null(json!(null), json!(0.01_f64))]
#[case::falsy_bool(json!(false), json!(0.01_f64))]
#[case::falsy_integer(json!(0_i32), json!(0.01_f64))]
#[case::falsy_float(json!(0.0_f64), json!(0.01_f64))]
#[case::falsy_text(json!(""), json!(0.01_f64))]
#[case::falsy_list(json!([]), json!(0.01_f64))]
#[case::falsy_object(json!({}), json!(0.01_f64))]
#[case::truthy_bool(json!(true), json!(0_i32))]
#[case::truthy_integer(json!(1_i32), json!(0_i32))]
#[case::truthy_negative_float(json!(-0.5_f64), json!(0_i32))]
#[case::truthy_text(json!("yes"), json!(0_i32))]
#[case::truthy_list(json!([0_i32]), json!(0_i32))]
#[case::truthy_object(
    serde_json::from_str(r#"{"k": 0}"#).expect("the flag"),
    json!(0_i32)
)]
fn the_allow_zero_temperature_flag_decides_a_zero_temperature(
    #[case] flag: Value,
    #[case] expected: Value,
) {
    let mut requested = Options::from([
        (String::from("temperature"), json!(0_i32)),
        (String::from("aws_sagemaker_allow_zero_temp"), flag.clone()),
    ]);
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("temperature"), Some(&expected), "{flag}");
}
