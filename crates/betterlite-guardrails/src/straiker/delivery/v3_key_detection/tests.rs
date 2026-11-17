#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;

use crate::straiker::delivery::fixtures::{capture_warnings, raw, settings};
use crate::straiker::delivery::settings::ApiVersion;

const ROUTING_WARNING: &str =
    "Straiker guardrail: api_version 'v1' cannot use an sk_agt_ key, routing to /api/v3/detect";

#[rstest]
#[case::unset(None, ApiVersion::V3)]
#[case::v3(Some("v3"), ApiVersion::V3)]
#[case::v1(Some("v1"), ApiVersion::V3)]
fn an_agent_key_always_routes_to_v3(
    #[case] configured: Option<&str>,
    #[case] expected: ApiVersion,
) {
    let mut args = raw();
    args.api_key = SecretString::from("sk_agt_live_key");
    args.api_version = configured.map(str::to_owned);
    assert_eq!(settings(&args).api_version, expected);
}

#[rstest]
#[case::sk_agt("sk_agt_x", ApiVersion::V3)]
#[case::uppercase("SK_AGT_x", ApiVersion::V1)]
#[case::no_underscore("sk_agt", ApiVersion::V1)]
#[case::not_a_prefix("xsk_agt_x", ApiVersion::V1)]
#[case::plain_key("sk_live_x", ApiVersion::V1)]
fn routing_follows_the_key_prefix_alone(#[case] key: &str, #[case] expected: ApiVersion) {
    let mut args = raw();
    args.api_key = SecretString::from(key);
    assert_eq!(settings(&args).api_version, expected);
}

#[test]
fn a_v1_declared_agent_key_warns_about_the_reroute() {
    let mut args = raw();
    args.api_key = SecretString::from("sk_agt_live_key");
    args.api_version = Some("v1".to_owned());
    let (warnings, _guard) = capture_warnings();
    let resolved = settings(&args);
    assert_eq!(resolved.api_version, ApiVersion::V3);
    assert_eq!(warnings.take(), vec![ROUTING_WARNING]);
}

#[rstest]
#[case::agent_key_unset("sk_agt_x", None, ApiVersion::V3)]
#[case::agent_key_v3("sk_agt_x", Some("v3"), ApiVersion::V3)]
#[case::plain_key_v1("sk_live_x", Some("v1"), ApiVersion::V1)]
#[case::plain_key_default("sk_live_x", None, ApiVersion::V1)]
fn no_warning_without_a_rerouted_v1_agent_key(
    #[case] key: &str,
    #[case] configured: Option<&str>,
    #[case] expected: ApiVersion,
) {
    let mut args = raw();
    args.api_key = SecretString::from(key);
    args.api_version = configured.map(str::to_owned);
    let (warnings, _guard) = capture_warnings();
    assert_eq!(settings(&args).api_version, expected);
    assert!(warnings.take().is_empty());
}
