#![cfg(test)]

use rstest::rstest;

use super::{CompatibleCallbacks, is_callback_compatible, substitute_env_variables};
use crate::generic_api_logger::tests::environment;
#[rstest]
#[case::sample_callback("sample_callback")]
#[case::rubrik("rubrik")]
#[case::sumologic("sumologic")]
#[case::qualifire_eval("qualifire_eval")]
#[case::ramp("ramp")]
fn embedded_configs_hold_each_named_callback(#[case] name: &str) {
    let callbacks = CompatibleCallbacks::embedded().expect("the embedded file parses");
    assert!(is_callback_compatible(&callbacks, name));
}
#[rstest]
#[case::upper_case("{{environment_variables.HOOK_URL}}/x", "https://hook.test/x")]
#[case::mixed_case("{{environment_variables.Foo1}}", "{{environment_variables.Foo1}}")]
#[case::lower_case("{{environment_variables.foo}}", "{{environment_variables.foo}}")]
#[case::unset("Bearer {{environment_variables.RAMP_API_KEY}}", "Bearer ")]
fn substitute_env_variables_replaces_only_upper_case_names(
    #[case] value: &str,
    #[case] expected: &str,
) {
    let env = environment(&[("HOOK_URL", "https://hook.test")]);
    let substituted = substitute_env_variables(value, &env).expect("the placeholder pattern");
    assert_eq!(substituted, expected);
}
