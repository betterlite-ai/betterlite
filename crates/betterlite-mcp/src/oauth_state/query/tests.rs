use rstest::rstest;

use super::append_query_params;

fn params(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::appends_two_params(
    "https://gw.example/callback",
    &[("code", "a b"), ("state", "x/y")],
    "https://gw.example/callback?code=a+b&state=x%2Fy"
)]
#[case::keeps_existing_pairs_and_fragment(
    "https://gw.example/callback?next=%2Fhome&blank=&flag#frag",
    &[("code", "a b")],
    "https://gw.example/callback?next=%2Fhome&blank=&flag=&code=a+b#frag"
)]
#[case::keeps_a_url_without_a_query("https://gw/cb", &[], "https://gw/cb")]
#[case::drops_an_empty_query_marker("https://gw.example/callback?", &[], "https://gw.example/callback")]
#[case::keeps_the_query_without_new_params("https://gw/cb?a=b", &[], "https://gw/cb?a=b")]
#[case::quotes_star_and_keeps_tilde(
    "https://gw/cb?x=1",
    &[("v", "~*")],
    "https://gw/cb?x=1&v=~%2A"
)]
#[case::keeps_path_params(
    "https://gw/cb;sid=1?q=2",
    &[("k", "v")],
    "https://gw/cb;sid=1?q=2&k=v"
)]
#[case::handles_a_relative_url("cb?x=1", &[("k", "v")], "cb?x=1&k=v")]
#[case::skips_an_empty_segment(
    "https://gw/cb?a=1&&b=2",
    &[],
    "https://gw/cb?a=1&b=2"
)]
#[case::round_trips_a_plus("https://gw/cb?q=a+b", &[], "https://gw/cb?q=a+b")]
#[case::appends_before_the_fragment(
    "https://gw/cb#frag",
    &[("k", "v")],
    "https://gw/cb?k=v#frag"
)]
#[case::drops_an_empty_query_marker_before_the_fragment(
    "https://gw/cb?#frag",
    &[],
    "https://gw/cb#frag"
)]
#[case::keeps_an_invalid_port(
    "https://gw:bad/cb",
    &[("k", "v")],
    "https://gw:bad/cb?k=v"
)]
#[case::appends_to_an_empty_path("https://gw", &[("k", "v")], "https://gw?k=v")]
#[case::appends_after_an_empty_path_query(
    "https://gw?x=1",
    &[("k", "v")],
    "https://gw?x=1&k=v"
)]
#[case::percent_encodes_the_reserved_chars(
    "https://gw/cb",
    &[("k", "a&b=c")],
    "https://gw/cb?k=a%26b%3Dc"
)]
#[case::encodes_a_plus_as_percent(
    "https://gw/cb",
    &[("k", "a+b")],
    "https://gw/cb?k=a%2Bb"
)]
#[case::encodes_a_space_and_a_plus_together(
    "https://gw/cb",
    &[("k", "a + b")],
    "https://gw/cb?k=a+%2B+b"
)]
#[case::decodes_and_reencodes_the_existing_query(
    "https://gw/cb?x=a%20b",
    &[],
    "https://gw/cb?x=a+b"
)]
#[case::keeps_a_semicolon_inside_a_value(
    "https://gw/cb?a=1;b=2",
    &[],
    "https://gw/cb?a=1%3Bb%3D2"
)]
#[case::keeps_an_invalid_percent_escape(
    "https://gw/cb?x=%zz",
    &[],
    "https://gw/cb?x=%25zz"
)]
#[case::keeps_a_blank_key("https://gw/cb?=v", &[], "https://gw/cb?=v")]
#[case::keeps_duplicate_keys(
    "https://gw/cb?k=1&k=2",
    &[("k", "3")],
    "https://gw/cb?k=1&k=2&k=3"
)]
#[case::lowercases_the_scheme(
    "HTTPS://GW.example/Cb?x=1",
    &[],
    "https://GW.example/Cb?x=1"
)]
#[case::keeps_a_custom_scheme_with_authority(
    "custom://gw/cb",
    &[("k", "v")],
    "custom://gw/cb?k=v"
)]
#[case::keeps_a_bracketed_ipv6_authority(
    "https://[::1]:8443/cb?x=1",
    &[("k", "v")],
    "https://[::1]:8443/cb?x=1&k=v"
)]
#[case::leaves_a_malformed_ipv6_url(
    "https://[::1/cb",
    &[("k", "v")],
    "https://[::1/cb"
)]
fn append_query_params_matches_litellm(
    #[case] url: &str,
    #[case] pairs: &[(&str, &str)],
    #[case] expected: &str,
) {
    assert_eq!(append_query_params(url, &params(pairs)), expected);
}
