use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::error::ZscalerAiGuardError;
use super::super::settings::{self, DEFAULT_TIMEOUT_DURATION, Settings};
use super::super::{
    API_KEY_ENV, DEFAULT_URL, POLICY_ID_ENV, SEND_ALIAS_ENV, SEND_TEAM_ID_ENV, SEND_USER_ID_ENV,
    URL_ENV,
};
use super::support::{args, captured, environment, guardrail};

#[test]
fn settings_use_the_argument_url_over_the_environment() {
    let mut args = args();
    args.api_base = Some("https://argument.example".to_owned());
    let resolved =
        Settings::resolve(&args, &environment(&[(URL_ENV, "https://env.example")])).unwrap();
    assert_eq!(resolved.url, "https://argument.example");
}

#[test]
fn settings_fall_back_to_the_url_environment_variable() {
    let resolved =
        Settings::resolve(&args(), &environment(&[(URL_ENV, "https://env.example")])).unwrap();
    assert_eq!(resolved.url, "https://env.example");
}

#[test]
fn settings_use_the_default_url_when_nothing_is_set() {
    let resolved = Settings::resolve(&args(), &environment(&[])).unwrap();
    assert_eq!(resolved.url, DEFAULT_URL);
}

#[test]
fn settings_ignore_an_empty_url_environment_variable() {
    let resolved = Settings::resolve(&args(), &environment(&[(URL_ENV, "")])).unwrap();
    assert_eq!(resolved.url, DEFAULT_URL);
}

#[test]
fn settings_keep_an_explicit_empty_url() {
    let mut args = args();
    args.api_base = Some(String::new());
    let resolved =
        Settings::resolve(&args, &environment(&[(URL_ENV, "https://env.example")])).unwrap();
    assert_eq!(resolved.url, "");
}

#[test]
fn settings_use_the_api_key_argument_over_the_environment() {
    let mut args = args();
    args.api_key = Some(SecretString::from("argument-key"));
    let resolved = Settings::resolve(&args, &environment(&[(API_KEY_ENV, "env-key")])).unwrap();
    assert_eq!(
        resolved.api_key.as_ref().unwrap().expose_secret(),
        "argument-key"
    );
}

#[test]
fn settings_fall_back_to_the_api_key_environment_variable() {
    let mut args = args();
    args.api_key = None;
    let resolved = Settings::resolve(&args, &environment(&[(API_KEY_ENV, "env-key")])).unwrap();
    assert_eq!(
        resolved.api_key.as_ref().unwrap().expose_secret(),
        "env-key"
    );
}

#[test]
fn settings_leave_a_missing_api_key_unset() {
    let mut args = args();
    args.api_key = None;
    let resolved = Settings::resolve(&args, &environment(&[(API_KEY_ENV, "")])).unwrap();
    assert!(resolved.api_key.is_none());
}

#[test]
fn settings_use_the_policy_id_argument_over_the_environment() {
    let mut args = args();
    args.policy_id = Some(5);
    let resolved = Settings::resolve(&args, &environment(&[(POLICY_ID_ENV, "9")])).unwrap();
    assert_eq!(resolved.policy_id, 5);
}

#[test]
fn settings_fall_back_to_the_policy_id_environment_variable() {
    let mut args = args();
    args.policy_id = None;
    let resolved = Settings::resolve(&args, &environment(&[(POLICY_ID_ENV, "9")])).unwrap();
    assert_eq!(resolved.policy_id, 9);
}

#[test]
fn settings_default_the_policy_id_to_minus_one() {
    let mut args = args();
    args.policy_id = None;
    let resolved = Settings::resolve(&args, &environment(&[(POLICY_ID_ENV, "")])).unwrap();
    assert_eq!(resolved.policy_id, -1);
}

#[test]
fn settings_refuse_a_non_integer_policy_id_environment_variable() {
    let mut args = args();
    args.policy_id = None;
    let error = Settings::resolve(&args, &environment(&[(POLICY_ID_ENV, "seven")])).unwrap_err();
    assert!(matches!(error, ZscalerAiGuardError::PolicyId { .. }));
    assert!(error.to_string().contains(POLICY_ID_ENV));
}

#[rstest]
#[case::argument_true(Some(true), None, true)]
#[case::argument_false_over_env(Some(false), Some("true"), false)]
#[case::env_true(None, Some("TRUE"), true)]
#[case::env_one(None, Some("1"), true)]
#[case::env_yes(None, Some("yes"), true)]
#[case::env_on(None, Some("on"), true)]
#[case::env_padded(None, Some(" true "), true)]
#[case::env_zero(None, Some("0"), false)]
#[case::env_false(None, Some("false"), false)]
#[case::env_empty(None, Some(""), false)]
#[case::env_absent(None, None, false)]
fn send_flags_resolve_from_the_argument_then_the_environment(
    #[case] argument: Option<bool>,
    #[case] environment_value: Option<&str>,
    #[case] expected: bool,
) {
    let mut args = args();
    args.send_user_api_key_alias = argument;
    let environment = environment_value.map_or_else(BTreeMap::new, |value| {
        environment(&[(SEND_ALIAS_ENV, value)])
    });
    let resolved = Settings::resolve(&args, &environment).unwrap();
    assert_eq!(resolved.send.alias, expected);
}

#[test]
fn settings_refuse_an_unparsable_send_flag() {
    let mut args = args();
    args.send_user_api_key_alias = None;
    let error = Settings::resolve(&args, &environment(&[(SEND_ALIAS_ENV, "maybe")])).unwrap_err();
    assert!(matches!(error, ZscalerAiGuardError::SendFlag { .. }));
    assert!(error.to_string().contains(SEND_ALIAS_ENV));
}

#[test]
fn a_false_send_flag_argument_beats_a_true_environment_value() {
    let mut args = args();
    args.send_user_api_key_alias = Some(false);
    let resolved = Settings::resolve(&args, &environment(&[(SEND_ALIAS_ENV, "true")])).unwrap();
    assert!(!resolved.send.alias);
}

#[test]
fn user_id_and_team_id_flags_read_their_own_environment_variables() {
    let mut args = args();
    args.send_user_api_key_user_id = None;
    args.send_user_api_key_team_id = None;
    let environment = environment(&[(SEND_USER_ID_ENV, "true"), (SEND_TEAM_ID_ENV, "1")]);
    let resolved = Settings::resolve(&args, &environment).unwrap();
    assert!(resolved.send.user_id);
    assert!(resolved.send.team_id);
}

#[test]
fn settings_keep_a_positive_timeout() {
    let mut args = args();
    args.timeout = Some(1.5);
    let resolved = Settings::resolve(&args, &environment(&[])).unwrap();
    assert_eq!(resolved.timeout, Duration::from_millis(1500));
}

#[test]
fn settings_default_the_timeout_when_it_is_absent() {
    let mut args = args();
    args.timeout = None;
    let resolved = Settings::resolve(&args, &environment(&[])).unwrap();
    assert_eq!(resolved.timeout, DEFAULT_TIMEOUT_DURATION);
}

#[test]
fn a_zero_timeout_falls_back_to_five_seconds() {
    let (timeout, _written) = captured(|| settings::resolve_timeout(Some(0.0)).unwrap());
    assert_eq!(timeout, DEFAULT_TIMEOUT_DURATION);
}

#[rstest]
#[case::zero(0.0)]
#[case::negative(-1.5)]
fn non_positive_timeouts_fall_back_and_name_the_value_and_default(#[case] value: f64) {
    let (timeout, written) = captured(|| settings::resolve_timeout(Some(value)).unwrap());
    assert_eq!(timeout, DEFAULT_TIMEOUT_DURATION);
    assert!(written.contains(&format!("timeout {value}")));
    assert!(written.contains("using 5 seconds"));
}

#[test]
fn settings_refuse_an_unrepresentable_timeout() {
    let error = settings::resolve_timeout(Some(f64::INFINITY)).unwrap_err();
    assert!(matches!(error, ZscalerAiGuardError::Timeout { .. }));
}

#[test]
fn update_params_takes_the_new_timeout() {
    let mut guardrail = guardrail("http://127.0.0.1:1");
    let params = decode_litellm_params(
        br#"{"guardrail": "zscaler_ai_guard", "mode": "pre_call", "timeout": 9.5}"#,
    )
    .unwrap();
    guardrail.update_params(&params).unwrap();
    assert_eq!(guardrail.settings.timeout, Duration::from_millis(9500));
}

#[test]
fn update_params_falls_back_on_a_non_positive_timeout() {
    let mut guardrail = guardrail("http://127.0.0.1:1");
    let params = decode_litellm_params(
        br#"{"guardrail": "zscaler_ai_guard", "mode": "pre_call", "timeout": -2.0}"#,
    )
    .unwrap();
    let (_result, written) = captured(|| guardrail.update_params(&params).unwrap());
    assert_eq!(guardrail.settings.timeout, DEFAULT_TIMEOUT_DURATION);
    assert!(written.contains("-2"));
}
