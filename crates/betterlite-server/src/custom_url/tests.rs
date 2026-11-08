#![cfg(test)]

use rstest::rstest;

use super::{CustomUrls, get_custom_url, join_paths};

#[rstest]
#[case::empty_base("", "v1/chat", "/v1/chat")]
#[case::empty_route("https://host", "", "https://host")]
#[case::both_empty("", "", "/")]
#[case::slashes_on_both_sides("https://host/", "/v1/chat", "https://host/v1/chat")]
#[case::tail_already_present("https://host/v1", "v1", "https://host/v1")]
#[case::similar_tail_is_appended("https://host/xv1", "v1", "https://host/xv1/v1")]
#[case::nested_route(
    "https://host/base",
    "/v1/chat/completions",
    "https://host/base/v1/chat/completions"
)]
fn join_paths_trims_slashes_and_joins(
    #[case] base: &str,
    #[case] route: &str,
    #[case] expected: &str,
) {
    assert_eq!(join_paths(base, route), expected);
}

#[rstest]
#[case::proxy_base_wins(
    Some("https://api.example.com"),
    "https://request.example.com",
    "",
    Some("/v1/chat"),
    "https://api.example.com/v1/chat"
)]
#[case::empty_proxy_base_still_wins(
    Some(""),
    "https://request.example.com",
    "",
    Some("/v1/chat"),
    "/v1/chat"
)]
#[case::request_base_when_unset(
    None,
    "https://request.example.com",
    "",
    Some("/v1/chat"),
    "https://request.example.com/v1/chat"
)]
#[case::root_path_then_route(
    Some("https://api.example.com"),
    "https://request.example.com",
    "/proxy",
    Some("/v1/chat"),
    "https://api.example.com/proxy/v1/chat"
)]
#[case::root_path_only(
    None,
    "https://request.example.com",
    "/proxy",
    None,
    "https://request.example.com/proxy"
)]
#[case::no_root_path(
    None,
    "https://request.example.com",
    "",
    Some("/v1/chat"),
    "https://request.example.com/v1/chat"
)]
#[case::request_base_ends_in_root_path(
    None,
    "https://request.example.com/tenant-a/",
    "/tenant-a",
    Some("/v1/chat"),
    "https://request.example.com/tenant-a/v1/chat"
)]
#[case::root_path_slash_adds_nothing(
    None,
    "https://request.example.com",
    "/",
    Some("/v1/chat"),
    "https://request.example.com/v1/chat"
)]
fn get_custom_url_picks_the_base_then_joins_root_path_and_route(
    #[case] proxy_base_url: Option<&str>,
    #[case] request_base_url: &str,
    #[case] request_root_path: &str,
    #[case] route: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        get_custom_url(proxy_base_url, request_base_url, request_root_path, route),
        expected
    );
}

#[test]
fn custom_urls_forward_with_the_configured_proxy_base() {
    let urls = CustomUrls {
        proxy_base_url: Some("https://api.example.com".to_owned()),
    };
    assert_eq!(
        urls.custom_url("https://request.example.com", "/proxy"),
        "https://api.example.com/proxy"
    );
    assert_eq!(
        urls.custom_route_url("https://request.example.com", "/proxy", "/v1/chat"),
        "https://api.example.com/proxy/v1/chat"
    );
}

#[test]
fn custom_urls_forward_without_a_configured_proxy_base() {
    let urls = CustomUrls {
        proxy_base_url: None,
    };
    assert_eq!(
        urls.custom_url("https://request.example.com", "/proxy"),
        "https://request.example.com/proxy"
    );
    assert_eq!(
        urls.custom_route_url("https://request.example.com", "/proxy", "/v1/chat"),
        "https://request.example.com/proxy/v1/chat"
    );
}
