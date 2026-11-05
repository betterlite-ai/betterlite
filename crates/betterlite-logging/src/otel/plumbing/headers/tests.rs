#![cfg(test)]
use super::parse_headers;
use rstest::rstest;

#[rstest]
#[case::empty(None, &[])]
#[case::blank(Some(""), &[])]
#[case::plain(Some("a=b,c=d"), &[("a", "b"), ("c", "d")])]
#[case::name_lowercased(Some("X-Key=Value"), &[("x-key", "Value")])]
#[case::percent_decoded(Some("authorization=Basic%20dG9rZW4%3D"), &[("authorization", "Basic dG9rZW4=")])]
#[case::plus_kept(Some("a=1+2"), &[("a", "1+2")])]
#[case::liberal_space(Some("authorization=Bearer abc def"), &[("authorization", "Bearer abc def")])]
#[case::liberal_keeps_percent(Some("a=x y%20z"), &[("a", "x y%20z")])]
#[case::equals_in_value(Some("a=b=c"), &[("a", "b=c")])]
#[case::spaces_around(Some(" a = b , c=d "), &[("a", "b"), ("c", "d")])]
#[case::last_wins(Some("a=1,a=2"), &[("a", "2")])]
#[case::no_equals_dropped(Some("novalue,a=b"), &[("a", "b")])]
#[case::empty_name_dropped(Some("=v,a=b"), &[("a", "b")])]
#[case::bad_name_dropped(Some("a b=v,c=d"), &[("c", "d")])]
#[case::empty_value_kept(Some("a="), &[("a", "")])]
#[case::semicolon_dropped(Some("a=b;c,d=e"), &[("d", "e")])]
fn exporter_headers_parse_like_the_sdk(
    #[case] raw: Option<&str>,
    #[case] expected: &[(&str, &str)],
) {
    let owned = expected
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    assert_eq!(parse_headers(raw), owned);
}
