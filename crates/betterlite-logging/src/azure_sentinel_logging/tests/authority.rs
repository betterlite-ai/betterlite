use rstest::rstest;

use super::support::{environment, required_environment};
use crate::azure_sentinel_logging::settings::{
    SentinelInit, SentinelSettings, normalize_authority_host, resolve_oauth_scope,
};

#[rstest]
#[case::bare_host("login.microsoftonline.us", "https://login.microsoftonline.us")]
#[case::trailing_slashes(
    "https://login.microsoftonline.us///",
    "https://login.microsoftonline.us"
)]
#[case::whitespace("  login.example.test/ ", "https://login.example.test")]
#[case::explicit_http("http://localhost:8080/", "http://localhost:8080")]
#[case::untouched(
    "https://login.microsoftonline.com",
    "https://login.microsoftonline.com"
)]
fn normalize_authority_host_adds_a_scheme_and_drops_trailing_slashes(
    #[case] given: &str,
    #[case] expected: &str,
) {
    assert_eq!(normalize_authority_host(given), expected);
}

#[rstest]
#[case::government(
    "https://login.microsoftonline.us",
    "https://monitor.azure.us/.default"
)]
#[case::government_with_port(
    "https://login.microsoftonline.us:443",
    "https://monitor.azure.us/.default"
)]
#[case::government_upper_case(
    "https://LOGIN.microsoftonline.us",
    "https://monitor.azure.us/.default"
)]
#[case::public(
    "https://login.microsoftonline.com",
    "https://monitor.azure.com/.default"
)]
#[case::unknown("https://login.example.test", "https://monitor.azure.com/.default")]
#[case::lookalike(
    "https://login.microsoftonline.us.example.test",
    "https://monitor.azure.com/.default"
)]
#[case::no_host("not a url", "https://monitor.azure.com/.default")]
fn resolve_oauth_scope_maps_the_authority_host_to_its_monitor_audience(
    #[case] authority: &str,
    #[case] scope: &str,
) {
    assert_eq!(resolve_oauth_scope(authority), scope);
}

#[test]
fn a_government_authority_host_sets_the_token_url_and_scope() {
    let mut variables = required_environment();
    variables.extend(environment(&[(
        "AZURE_SENTINEL_AUTHORITY_HOST",
        "login.microsoftonline.us/",
    )]));
    let settings = SentinelSettings::resolve(&SentinelInit::default(), &variables)
        .expect("the environment resolves");
    assert_eq!(
        settings.token_url,
        "https://login.microsoftonline.us/tenant-1/oauth2/v2.0/token"
    );
    assert_eq!(settings.scope, "https://monitor.azure.us/.default");
}
