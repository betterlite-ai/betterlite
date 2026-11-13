#![cfg(test)]

use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{MapParams, SUPPORTED_PARAMS, SUPPORTED_RESPONSE_FORMATS, map_openai_params_stt};

fn object(text: &str) -> Map<String, Value> {
    serde_json::from_str(text).expect("a JSON object")
}

fn mapped(
    non_default: &str,
    optional: &str,
    drop_params: bool,
    global_drop: bool,
) -> Map<String, Value> {
    map_openai_params_stt(&MapParams {
        non_default: &object(non_default),
        optional_params: &object(optional),
        drop_params,
        global_drop_params: global_drop,
    })
    .expect("the mapped params")
}

#[test]
fn the_supported_transcription_params_are_listed_in_order() {
    assert_eq!(
        SUPPORTED_PARAMS,
        ["prompt", "response_format", "temperature", "language"]
    );
}

#[test]
fn the_supported_response_formats_are_the_three_groq_formats() {
    assert_eq!(SUPPORTED_RESPONSE_FORMATS, ["json", "verbose_json", "text"]);
}

#[test]
fn every_non_default_param_is_copied_over_the_existing_ones() {
    assert_eq!(
        mapped(
            r#"{"temperature":0,"language":"en"}"#,
            r#"{"prompt":"keep"}"#,
            false,
            false
        ),
        object(r#"{"prompt":"keep","temperature":0,"language":"en"}"#)
    );
}

#[rstest]
#[case::json("json")]
#[case::verbose_json("verbose_json")]
#[case::text("text")]
fn a_supported_response_format_is_copied(#[case] format: &str) {
    let expected = json!({"response_format": format})
        .as_object()
        .cloned()
        .expect("an object");
    assert_eq!(
        mapped(
            &format!(r#"{{"response_format":"{format}"}}"#),
            "{}",
            false,
            false
        ),
        expected
    );
}

#[rstest]
#[case::call_flag(false, true)]
#[case::global_flag(true, false)]
fn either_drop_flag_drops_an_unsupported_response_format(
    #[case] global_drop: bool,
    #[case] drop_params: bool,
) {
    assert_eq!(
        mapped(
            r#"{"response_format":"srt","temperature":2}"#,
            "{}",
            drop_params,
            global_drop
        ),
        object(r#"{"temperature":2}"#)
    );
}

#[test]
fn neither_drop_flag_leaves_the_unsupported_format_failing() {
    let error = map_openai_params_stt(&MapParams {
        non_default: &object(r#"{"response_format":"srt"}"#),
        optional_params: &object("{}"),
        drop_params: false,
        global_drop_params: false,
    })
    .expect_err("the unsupported format");
    assert_eq!(error.status_code(), 400);
    assert_eq!(
        error.to_string(),
        "Groq doesn't support response_format=srt. To drop unsupported openai params from the call, set `litellm.drop_params = True`"
    );
}

#[test]
fn a_non_string_response_format_is_unsupported_too() {
    let error = map_openai_params_stt(&MapParams {
        non_default: &object(r#"{"response_format":3}"#),
        optional_params: &object("{}"),
        drop_params: false,
        global_drop_params: false,
    })
    .expect_err("the unsupported format");
    assert!(error.to_string().contains("response_format=3"));
}

#[test]
fn an_existing_optional_value_is_overwritten_by_the_non_default_one() {
    assert_eq!(
        mapped(r#"{"prompt":"new"}"#, r#"{"prompt":"old"}"#, false, false),
        object(r#"{"prompt":"new"}"#)
    );
}
