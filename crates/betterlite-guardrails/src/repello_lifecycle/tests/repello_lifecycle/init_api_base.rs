use betterlite_guardrails::repello_lifecycle::DEFAULT_API_BASE;
use betterlite_guardrails::repello_lifecycle::config::{RepelloAIParams, RepelloAISettings};
use secrecy::SecretString;

use crate::support::{empty_env, lookup, params};

fn params_with_base(api_base: Option<&str>) -> RepelloAIParams {
    let mut configured = params();
    configured.api_key = Some(SecretString::from(String::from("explicit")));
    configured.api_base = api_base.map(str::to_owned);
    configured
}

#[test]
fn the_parameter_wins_over_the_environment_and_the_default() {
    let env = lookup(&[("REPELLOAI_API_BASE", "http://env.test")]);
    let settings =
        RepelloAISettings::resolve(&params_with_base(Some("http://param.test")), &env).unwrap();
    assert_eq!(settings.api_base, "http://param.test");
}

#[test]
fn the_environment_name_is_the_second_source() {
    let env = lookup(&[("REPELLOAI_API_BASE", "http://env.test")]);
    let settings = RepelloAISettings::resolve(&params_with_base(None), &env).unwrap();
    assert_eq!(settings.api_base, "http://env.test");
}

#[test]
fn the_default_base_is_the_argus_sdk_base() {
    let settings = RepelloAISettings::resolve(&params_with_base(None), &empty_env()).unwrap();
    assert_eq!(settings.api_base, DEFAULT_API_BASE);
    assert_eq!(settings.api_base, "https://argusapi.repello.ai/sdk/v1");
}

#[test]
fn an_empty_parameter_falls_through_to_the_environment() {
    let env = lookup(&[("REPELLOAI_API_BASE", "http://env.test")]);
    let settings = RepelloAISettings::resolve(&params_with_base(Some("")), &env).unwrap();
    assert_eq!(settings.api_base, "http://env.test");
}

#[test]
fn an_empty_environment_name_falls_through_to_the_default() {
    let env = lookup(&[("REPELLOAI_API_BASE", "")]);
    let settings = RepelloAISettings::resolve(&params_with_base(None), &env).unwrap();
    assert_eq!(settings.api_base, DEFAULT_API_BASE);
}

#[test]
fn a_trailing_slash_is_kept_as_given() {
    let settings =
        RepelloAISettings::resolve(&params_with_base(Some("http://param.test/")), &empty_env())
            .unwrap();
    assert_eq!(settings.api_base, "http://param.test/");
}
