use crate::databricks_auth::url_parts::{path, workspace};
use rstest::rstest;

#[rstest]
#[case::empty("", "")]
#[case::root("/", "/")]
#[case::host_only("https://host", "")]
#[case::host_slash("https://host/", "/")]
#[case::host_path("https://host/a/b", "/a/b")]
#[case::query_after_host("https://host?x=1", "")]
#[case::query_after_path("https://host/a?x=1", "/a")]
#[case::fragment("https://host/a#frag", "/a")]
#[case::double_slash_path("https://host//x", "//x")]
#[case::protocol_relative("//host/x", "/x")]
#[case::protocol_relative_only("//host", "")]
#[case::not_a_url("host/base", "host/base")]
#[case::scheme_late("a/b://c", "a/b://c")]
#[case::scheme_symbols("h-t.t+p://host/x", "/x")]
#[case::scheme_starts_with_digit("1http://host/x", "1http://host/x")]
fn the_path_follows_python_urlsplit(#[case] url: &str, #[case] expected: &str) {
    assert_eq!(path(url), expected);
}

#[rstest]
#[case::host(
    "https://adb-1.databricks.net/serving-endpoints",
    "https://adb-1.databricks.net"
)]
#[case::port(
    "https://adb-1.databricks.net:8443/base?q=1#f",
    "https://adb-1.databricks.net:8443"
)]
#[case::userinfo("https://user:pass@host/x", "https://user:pass@host")]
#[case::scheme_case("HTTP://HOST/x", "http://HOST")]
#[case::no_scheme("host/base", "")]
#[case::protocol_relative("//host/x", "//host")]
fn the_workspace_is_the_scheme_and_authority(#[case] url: &str, #[case] expected: &str) {
    assert_eq!(workspace(url), expected);
}
