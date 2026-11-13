#![cfg(test)]

use serde_json::{Value, json};

use super::{Options, SUPPORTED_OPENAI_PARAMS, map_openai_params};

fn options(pairs: (&str, Value)) -> Options {
    Options::from([(pairs.0.to_owned(), pairs.1)])
}

#[test]
fn a_zero_temperature_is_rewritten_to_the_positive_floor() {
    let mut requested = options(("temperature", json!(0.0)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("temperature"), Some(&json!(0.01)));
}

#[test]
fn an_integer_zero_temperature_is_rewritten_to_the_positive_floor() {
    let mut requested = options(("temperature", json!(0)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("temperature"), Some(&json!(0.01)));
}

#[test]
fn a_positive_temperature_is_kept_as_given() {
    let mut requested = options(("temperature", json!(0.7)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("temperature"), Some(&json!(0.7)));
}

#[test]
fn the_allow_zero_temperature_flag_keeps_a_zero_temperature() {
    let mut requested = Options::from([
        (String::from("temperature"), json!(0)),
        (String::from("aws_sagemaker_allow_zero_temp"), json!(true)),
    ]);
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("temperature"), Some(&json!(0)));
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
    let mut requested = options(("n", json!(3)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("best_of"), Some(&json!(3)));
    assert_eq!(mapped.get("do_sample"), Some(&json!(true)));
}

#[test]
fn a_zero_max_tokens_becomes_one() {
    let mut requested = options(("max_tokens", json!(0)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("max_new_tokens"), Some(&json!(1)));
}

#[test]
fn a_positive_max_tokens_is_kept_as_given() {
    let mut requested = options(("max_tokens", json!(64)));
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("max_new_tokens"), Some(&json!(64)));
}

#[test]
fn max_tokens_after_max_completion_tokens_wins() {
    let mut requested = Options::from([
        (String::from("max_completion_tokens"), json!(70)),
        (String::from("max_tokens"), json!(0)),
    ]);
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("max_new_tokens"), Some(&json!(1)));
}

#[test]
fn max_completion_tokens_after_max_tokens_wins() {
    let mut requested = Options::from([
        (String::from("max_tokens"), json!(0)),
        (String::from("max_completion_tokens"), json!(70)),
    ]);
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("max_new_tokens"), Some(&json!(70)));
}

#[test]
fn stop_top_p_and_stream_pass_through_as_given() {
    let mut requested = Options::from([
        (String::from("stop"), json!(["END"])),
        (String::from("top_p"), json!(0.9)),
        (String::from("stream"), json!(false)),
    ]);
    let mut mapped = Options::new();
    map_openai_params(&mut requested, &mut mapped);
    assert_eq!(mapped.get("stop"), Some(&json!(["END"])));
    assert_eq!(mapped.get("top_p"), Some(&json!(0.9)));
    assert_eq!(mapped.get("stream"), Some(&json!(false)));
}

#[test]
fn an_option_outside_the_supported_list_is_not_mapped() {
    let mut requested = options(("frequency_penalty", json!(1)));
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

#[test]
fn a_falsy_allow_zero_temperature_flag_still_rewrites_a_zero_temperature() {
    for flag in [
        json!(null),
        json!(false),
        json!(0),
        json!(0.0),
        json!(""),
        json!([]),
        json!({}),
    ] {
        let mut requested = Options::from([
            (String::from("temperature"), json!(0)),
            (String::from("aws_sagemaker_allow_zero_temp"), flag.clone()),
        ]);
        let mut mapped = Options::new();
        map_openai_params(&mut requested, &mut mapped);
        assert_eq!(mapped.get("temperature"), Some(&json!(0.01)), "{flag}");
    }
}

#[test]
fn a_truthy_allow_zero_temperature_flag_keeps_a_zero_temperature() {
    for flag in [
        json!(true),
        json!(1),
        json!(-0.5),
        json!("yes"),
        json!([0]),
        json!({"k": 0}),
    ] {
        let mut requested = Options::from([
            (String::from("temperature"), json!(0)),
            (String::from("aws_sagemaker_allow_zero_temp"), flag.clone()),
        ]);
        let mut mapped = Options::new();
        map_openai_params(&mut requested, &mut mapped);
        assert_eq!(mapped.get("temperature"), Some(&json!(0)), "{flag}");
    }
}
