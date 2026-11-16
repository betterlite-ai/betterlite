use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_guardrails::repello_lifecycle::config::RepelloAIParams;
use rstest::rstest;

use crate::support::{empty_env, lookup, params};

fn params_with_fallback(fallback: UnreachableFallback, default_on: bool) -> RepelloAIParams {
    let mut configured = params();
    configured.unreachable_fallback = fallback;
    configured.default_on = default_on;
    configured
}

#[rstest]
#[case::fail_closed(UnreachableFallback::FailClosed)]
#[case::fail_open(UnreachableFallback::FailOpen)]
fn every_fallback_mode_reaches_the_settings(#[case] fallback: UnreachableFallback) {
    let env = lookup(&[("ARGUS_API_KEY", "env-key")]);
    let settings = betterlite_guardrails::repello_lifecycle::config::RepelloAISettings::resolve(
        &params_with_fallback(fallback, true),
        &env,
    )
    .unwrap();
    assert_eq!(settings.unreachable_fallback, fallback);
}

#[test]
fn the_config_default_is_fail_closed() {
    assert_eq!(
        UnreachableFallback::default(),
        UnreachableFallback::FailClosed
    );
}

#[rstest]
#[case::missing_default(true)]
#[case::explicit_false(false)]
fn the_default_on_flag_comes_from_the_params(#[case] default_on: bool) {
    let configured = params_with_fallback(UnreachableFallback::FailClosed, default_on);
    assert_eq!(configured.default_on, default_on);
}
