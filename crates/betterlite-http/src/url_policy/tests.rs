#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use url::Url;

use crate::url_policy::base::ConfiguredBase;
use crate::url_policy::destination::{candidates, destination_allowed};
use crate::url_policy::origin::assert_same_origin;

#[derive(Clone, Copy)]
enum Behavior {
    Origin(&'static str, &'static str),
    Destination(&'static str, &'static [&'static str]),
    Base(&'static str),
    Candidates(&'static str),
}

fn origin_outcome(candidate: &str, expected: &str) -> String {
    match assert_same_origin(candidate, expected) {
        Ok(origin) => format!("ok|{}|{}|{}", origin.scheme, origin.host, origin.port),
        Err(error) => error.to_string(),
    }
}

fn destination_outcome(url: &str, entries: &[&str]) -> String {
    let allowed: Vec<AllowedDestination> = entries
        .iter()
        .filter_map(|entry| AllowedDestination::parse(entry))
        .collect();
    destination_allowed(url, &allowed).to_string()
}

fn base_outcome(url: &str) -> String {
    let parsed = Url::parse(url).unwrap();
    match ConfiguredBase::parse(parsed, "litellm_params.api_base") {
        Ok(base) => format!(
            "{}|{}",
            base.url,
            base.authorization.map_or_else(
                || "-".to_owned(),
                |secret| secret.expose_secret().to_owned()
            )
        ),
        Err(error) => error.to_string(),
    }
}

fn observe(behavior: Behavior) -> String {
    match behavior {
        Behavior::Origin(candidate, expected) => origin_outcome(candidate, expected),
        Behavior::Destination(url, entries) => destination_outcome(url, entries),
        Behavior::Base(url) => base_outcome(url),
        Behavior::Candidates(value) => candidates(value).join("|"),
    }
}

#[rstest]
#[case::an_explicit_default_port_matches(
    Behavior::Origin(
        "https://api.example.com:443/v1/jobs",
        "https://api.example.com/v1/jobs"
    ),
    "ok|https|api.example.com|443"
)]
#[case::host_case_and_trailing_dots_fold(
    Behavior::Origin("https://API.Example.COM./v1/jobs", "https://api.example.com/v1/jobs"),
    "ok|https|api.example.com|443"
)]
#[case::an_http_default_port_is_80(
    Behavior::Origin("http://api.example.com/v1/jobs", "http://api.example.com:80/v1/jobs"),
    "ok|http|api.example.com|80"
)]
#[case::ipv6_literals_compare_by_address(
    Behavior::Origin("https://[::1]:8443/v1/jobs", "https://[::1]:8443/v1/jobs"),
    "ok|https|::1|8443"
)]
#[case::an_ipv6_default_port_is_443(
    Behavior::Origin("https://[::1]/v1/jobs", "https://[::1]:443/v1/jobs"),
    "ok|https|::1|443"
)]
#[case::an_ipv6_zone_folds_like_the_host(
    Behavior::Origin("https://[fe80::1%Eth0]/v1/jobs", "https://[fe80::1%eth0]/v1/jobs"),
    "ok|https|fe80::1%eth0|443"
)]
#[case::userinfo_does_not_change_the_origin(
    Behavior::Origin(
        "https://user:pass@api.example.com/v1/jobs",
        "https://api.example.com/v1/jobs"
    ),
    "ok|https|api.example.com|443"
)]
#[case::a_scheme_mismatch_is_refused(
    Behavior::Origin("https://api.example.com/v1/jobs", "http://api.example.com/v1/jobs"),
    "Origin mismatch on scheme"
)]
#[case::a_port_mismatch_is_refused(
    Behavior::Origin(
        "https://api.example.com:8443/v1/jobs",
        "https://api.example.com/v1/jobs"
    ),
    "Origin mismatch on port"
)]
#[case::a_host_suffix_is_refused(
    Behavior::Origin(
        "https://api.example.com.evil.test/v1/jobs",
        "https://api.example.com/v1/jobs"
    ),
    "Origin mismatch on host"
)]
#[case::a_relative_candidate_has_no_scheme(
    Behavior::Origin("//api.example.com/v1/jobs", "https://api.example.com/v1/jobs"),
    "URL scheme is not allowed"
)]
#[case::only_http_and_https_schemes_are_allowed(
    Behavior::Origin("ftp://api.example.com/v1/jobs", "https://api.example.com/v1/jobs"),
    "URL scheme is not allowed"
)]
#[case::an_invalid_candidate_port_is_a_value_error(
    Behavior::Origin(
        "https://api.example.com:8o/v1/jobs",
        "https://api.example.com/v1/jobs"
    ),
    "Port could not be cast to integer value as '8o'"
)]
#[case::an_out_of_range_port_is_a_value_error(
    Behavior::Origin(
        "https://api.example.com:99999/v1/jobs",
        "https://api.example.com/v1/jobs"
    ),
    "Port out of range 0-65535"
)]
#[case::a_bare_host_allows_any_port(Behavior::Destination("https://api.example.com/v1", &["api.example.com"]), "true")]
#[case::a_scheme_entry_pins_the_scheme(Behavior::Destination("http://api.example.com/v1", &["https://api.example.com"]), "false")]
#[case::an_https_entry_uses_port_443(Behavior::Destination("https://api.example.com/v1", &["https://api.example.com"]), "true")]
#[case::a_port_entry_pins_the_port(Behavior::Destination("https://api.example.com:8443/v1", &["api.example.com:8443"]), "true")]
#[case::a_default_port_entry_pins_it(Behavior::Destination("http://api.example.com/v1", &["api.example.com:80"]), "true")]
#[case::an_https_url_uses_port_443(Behavior::Destination("https://api.example.com/v1", &["api.example.com:443"]), "true")]
#[case::entry_case_and_dots_fold(Behavior::Destination("https://API.EXAMPLE.COM./v1", &["api.example.com"]), "true")]
#[case::ipv6_entries_match(Behavior::Destination("https://[::1]:8080/v1", &["[::1]:8080"]), "true")]
#[case::an_ipv6_zone_folds_in_the_allowlist(Behavior::Destination("https://[fe80::1%Eth0]/v1", &["[FE80::1%eth0]"]), "true")]
#[case::a_host_suffix_is_not_allowlisted(Behavior::Destination("https://api.example.com.evil.test/v1", &["api.example.com"]), "false")]
#[case::a_credential_url_is_refused(Behavior::Destination("https://user:pass@api.example.com/v1", &["api.example.com"]), "false")]
#[case::a_relative_url_is_refused(Behavior::Destination("/v1/models", &["api.example.com"]), "false")]
#[case::a_non_http_scheme_is_refused(Behavior::Destination("ftp://api.example.com/v1", &["api.example.com"]), "false")]
#[case::an_empty_allowlist_allows_nothing(Behavior::Destination("https://api.example.com/v1", &[]), "false")]
#[case::userinfo_becomes_basic_auth(
    Behavior::Base("http://user:pass@api.example.com/v1?x=1"),
    "http://api.example.com/v1?x=1|Basic dXNlcjpwYXNz"
)]
#[case::percent_encoded_userinfo_is_decoded(
    Behavior::Base("http://us%40er:p%40ss@api.example.com/v1"),
    "http://api.example.com/v1|Basic dXNAZXI6cEBzcw=="
)]
#[case::a_password_may_be_missing(
    Behavior::Base("http://user:@api.example.com/v1"),
    "http://api.example.com/v1|Basic dXNlcjo="
)]
#[case::https_is_accepted(
    Behavior::Base("https://api.example.com/v1"),
    "https://api.example.com/v1|-"
)]
#[case::a_loopback_base_is_accepted(
    Behavior::Base("http://127.0.0.1:8080/v1"),
    "http://127.0.0.1:8080/v1|-"
)]
#[case::a_private_base_is_accepted(Behavior::Base("http://10.1.2.3/v1"), "http://10.1.2.3/v1|-")]
#[case::another_scheme_is_refused_by_key(
    Behavior::Base("ftp://api.example.com/v1"),
    "litellm_params.api_base uses the ftp scheme, and only http and https are supported"
)]
#[case::host_and_path_candidates(
    Behavior::Candidates("https://a.example.com/api/v1,https://b.example.com/y"),
    "https://a.example.com/api/v1|/a.example.com/api/v1|https://b.example.com/y|/b.example.com/y"
)]
#[case::candidates_are_trimmed(
    Behavior::Candidates(" a.example.com/v1 , b.example.com "),
    "a.example.com/v1|v1|b.example.com"
)]
#[case::empty_candidates_are_dropped(Behavior::Candidates(",,"), "")]
fn http_boundaries_url_boundary_tests_behavior(#[case] behavior: Behavior, #[case] expected: &str) {
    assert_eq!(observe(behavior), expected);
}

#[test]
fn refuses_inputs_past_the_length_bound() {
    let long_url = format!("https://api.example.com/{}", "a".repeat(0x0001_0000));
    let error = assert_same_origin(&long_url, "https://api.example.com/v1").unwrap_err();
    assert_eq!(error.to_string(), "the URL is longer than 65536 characters");
    assert!(!destination_allowed(&long_url, &[]));
    assert!(AllowedDestination::parse(&long_url).is_none());
}

#[rstest]
fn replays_the_url_policy_corpus(#[files("fuzz/corpus/url_policy/*")] path: PathBuf) {
    let text = fs::read_to_string(path).unwrap();
    let mut lines = text.lines();
    let candidate = lines.next().unwrap_or_default();
    let expected = lines.next().unwrap_or_default();
    let _origin = assert_same_origin(candidate, expected);
    let url = lines.next().unwrap_or_default();
    let allowed: Vec<AllowedDestination> = lines.filter_map(AllowedDestination::parse).collect();
    let _allowed = destination_allowed(url, &allowed);
    let _candidates = candidates(url);
    let _base = Url::parse(url)
        .ok()
        .and_then(|parsed| ConfiguredBase::parse(parsed, "api_base").ok());
}
