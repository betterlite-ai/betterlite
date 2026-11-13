#![cfg(test)]

use rstest::rstest;

use super::is_protected;

#[rstest]
#[case::authorization("authorization")]
#[case::api_key("api-key")]
#[case::x_api_key("x-api-key")]
#[case::x_goog_api_key("x-goog-api-key")]
#[case::ocp_apim_subscription_key("ocp-apim-subscription-key")]
#[case::host("host")]
#[case::content_length("content-length")]
#[case::accept_encoding("accept-encoding")]
fn the_eight_literals_are_protected(#[case] name: &str) {
    assert!(is_protected(name));
}

#[rstest]
#[case::title_case("Authorization")]
#[case::upper_case("X-API-KEY")]
#[case::mixed_case("Ocp-Apim-Subscription-Key")]
fn literal_matching_ignores_case(#[case] name: &str) {
    assert!(is_protected(name));
}

#[rstest]
#[case::date("x-amz-date")]
#[case::security_token("X-Amz-Security-Token")]
#[case::prefix_alone("x-amz-")]
#[case::mixed_prefix("x-AMZ-foo")]
fn the_x_amz_prefix_is_protected(#[case] name: &str) {
    assert!(is_protected(name));
}

#[rstest]
#[case::prefix_without_dash("x-amz")]
#[case::content_type("content-type")]
#[case::literal_with_suffix("authorization-extra")]
#[case::literal_with_prefix("x-authorization")]
#[case::api_key_plural("api-keys")]
#[case::empty("")]
fn other_names_are_forwardable(#[case] name: &str) {
    assert!(!is_protected(name));
}
