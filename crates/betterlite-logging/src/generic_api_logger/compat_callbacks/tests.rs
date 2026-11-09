#![cfg(test)]

use rstest::rstest;

use super::{CompatibleCallbacks, get_callback_config, substitute_env_variables};
use crate::generic_api_logger::tests::environment;

#[rstest]
#[case::sample_callback("sample_callback")]
#[case::rubrik("rubrik")]
#[case::sumologic("sumologic")]
#[case::qualifire_eval("qualifire_eval")]
#[case::ramp("ramp")]
fn embedded_configs_hold_each_named_callback(#[case] name: &str) {
    let callbacks = CompatibleCallbacks::embedded().expect("the embedded file parses");
    assert!(callbacks.configs.contains_key(name));
    assert!(get_callback_config(name).is_some());
}

#[test]
fn invalid_config_text_is_an_error() {
    CompatibleCallbacks::parse("not json").unwrap_err();
}

#[rstest]
#[case::upper_case("{{environment_variables.HOOK_URL}}/x", "https://hook.test/x")]
#[case::mixed_case("{{environment_variables.Foo1}}", "{{environment_variables.Foo1}}")]
#[case::lower_case("{{environment_variables.foo}}", "{{environment_variables.foo}}")]
#[case::other_prefix("{{variables.HOOK_URL}}", "{{variables.HOOK_URL}}")]
#[case::unset("Bearer {{environment_variables.RAMP_API_KEY}}", "Bearer ")]
fn substitute_env_variables_replaces_only_upper_case_names(
    #[case] value: &str,
    #[case] expected: &str,
) {
    let env = environment(&[("HOOK_URL", "https://hook.test")]);
    assert_eq!(substitute_env_variables(value, &env), expected);
}
