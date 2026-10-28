#![cfg(test)]

use super::*;
use rstest::rstest;

#[rstest]
#[case::kill_url_absolute_http_https("http://example.com")]
#[case::kill_url_https("https://example.com")]
fn kill_url_accepted(#[case] input: &str) {
    assert_eq!(KillSwitchUrl::try_from(input.to_owned()).unwrap().0, input);
}

#[rstest]
#[case::kill_url_bad_authority("http://[::1")]
#[case::kill_url_missing_authority("http:example.com")]
#[case::kill_url_one_slash("http:/example.com")]
#[case::kill_url_backslash(r"http:\example.com")]
fn kill_url_rejected(#[case] input: &str) {
    KillSwitchUrl::try_from(input.to_owned()).unwrap_err();
}

#[rstest]
#[case::userinfo_ipv6("http://user@[::1]/", true)]
#[case::invalid_ip_literal("http://[abc]", false)]
#[case::bracket_suffix("http://[::1]x", false)]
#[case::scheme_tab("ht\ttp://example.com", true)]
#[case::authority_tab("http:/\t/example.com", true)]
#[case::cr_lf("h\rtt\np://example.com", true)]
#[case::ipvfuture("http://[v1.a]", true)]
#[case::invalid_ipvfuture("http://[v.a]", false)]
#[case::scoped_ipv6("http://[fe80::1%eth0]", true)]
#[case::invalid_scope("http://[fe80::1%]", false)]
#[case::nfkc_separator("http://example.com\u{ff0f}bad", false)]
fn urlsplit_acceptance(#[case] input: &str, #[case] accepted: bool) {
    let result = KillSwitchUrl::try_from(input.to_owned());
    assert_eq!(result.is_ok(), accepted);
    if let Ok(url) = result {
        assert_eq!(url.0, input);
    }
}
