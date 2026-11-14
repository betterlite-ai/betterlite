use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use crate::health_probe_settings::model_settings::{HealthSettingsError, ModelHealthSettings};
use crate::health_probe_settings::tests::support::object;

fn unset() -> ModelHealthSettings {
    ModelHealthSettings {
        mode: None,
        max_tokens: None,
        max_tokens_reasoning: None,
        max_tokens_non_reasoning: None,
        supports_max_tokens: None,
        reasoning_effort: None,
        model: None,
        voice: None,
        params: None,
    }
}

#[test]
fn parse_reads_every_health_check_knob() {
    let model_info = object(
        r#"{
            "mode": "chat",
            "health_check_max_tokens": 12,
            "health_check_max_tokens_reasoning": 64,
            "health_check_max_tokens_non_reasoning": 8,
            "health_check_supports_max_tokens": true,
            "health_check_reasoning_effort": "none",
            "health_check_model": "override-model",
            "health_check_voice": "echo",
            "health_check_params": {"mediaSource": "x"}
        }"#,
    );
    let mut expected = unset();
    expected.mode = Some("chat".to_owned());
    expected.max_tokens = Some(12);
    expected.max_tokens_reasoning = Some(64);
    expected.max_tokens_non_reasoning = Some(8);
    expected.supports_max_tokens = Some(true);
    expected.reasoning_effort = Some(Json::Text("none".to_owned()));
    expected.model = Some("override-model".to_owned());
    expected.voice = Some("echo".to_owned());
    expected.params = Some(object(r#"{"mediaSource":"x"}"#));
    assert_eq!(ModelHealthSettings::parse(&model_info).unwrap(), expected);
}

#[test]
fn null_knobs_are_unset() {
    let model_info = object(
        r#"{
            "mode": null,
            "health_check_max_tokens": null,
            "health_check_max_tokens_reasoning": null,
            "health_check_max_tokens_non_reasoning": null,
            "health_check_supports_max_tokens": null,
            "health_check_reasoning_effort": null,
            "health_check_model": null,
            "health_check_voice": null,
            "health_check_params": null
        }"#,
    );
    assert_eq!(ModelHealthSettings::parse(&model_info).unwrap(), unset());
}

#[rstest]
#[case::a_number(r#"{"mode":3}"#)]
#[case::a_list(r#"{"mode":["chat"]}"#)]
fn a_non_text_mode_is_unset(#[case] model_info: &str) {
    assert_eq!(
        ModelHealthSettings::parse(&object(model_info))
            .unwrap()
            .mode,
        None
    );
}

#[test]
fn a_quoted_integer_limit_is_read_as_an_integer() {
    let settings =
        ModelHealthSettings::parse(&object(r#"{"health_check_max_tokens":" 16 "}"#)).unwrap();
    assert_eq!(settings.max_tokens, Some(16));
}

#[rstest]
#[case::max_tokens(
    r#"{"health_check_max_tokens":"many"}"#,
    "health_check_max_tokens",
    "an integer"
)]
#[case::a_fraction(
    r#"{"health_check_max_tokens_reasoning":1.5}"#,
    "health_check_max_tokens_reasoning",
    "an integer"
)]
#[case::non_reasoning(
    r#"{"health_check_max_tokens_non_reasoning":true}"#,
    "health_check_max_tokens_non_reasoning",
    "an integer"
)]
#[case::supports_max_tokens(
    r#"{"health_check_supports_max_tokens":"yes"}"#,
    "health_check_supports_max_tokens",
    "a boolean"
)]
#[case::model(r#"{"health_check_model":5}"#, "health_check_model", "a string")]
#[case::voice(r#"{"health_check_voice":[]}"#, "health_check_voice", "a string")]
fn a_knob_of_the_wrong_type_names_its_field(
    #[case] model_info: &str,
    #[case] field: &'static str,
    #[case] expected: &'static str,
) {
    assert_eq!(
        ModelHealthSettings::parse(&object(model_info)).unwrap_err(),
        HealthSettingsError::Invalid { field, expected }
    );
}

#[test]
fn a_malformed_health_check_params_is_ignored() {
    let settings = ModelHealthSettings::parse(&object(r#"{"health_check_params":"oops"}"#));
    assert_eq!(settings.unwrap().params, None);
}

#[test]
fn health_check_params_are_kept_with_every_key() {
    let settings = ModelHealthSettings::parse(&object(
        r#"{"health_check_params":{"mediaSource":"x","n":1}}"#,
    ));
    assert_eq!(
        settings.unwrap().params,
        Some(object(r#"{"mediaSource":"x","n":1}"#))
    );
}

#[test]
fn reasoning_effort_keeps_a_value_litellm_would_forward() {
    let settings =
        ModelHealthSettings::parse(&object(r#"{"health_check_reasoning_effort":0}"#)).unwrap();
    assert_eq!(
        settings.reasoning_effort,
        Some(Json::Number(Number::from(0_i64)))
    );
}
