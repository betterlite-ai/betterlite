#![cfg(test)]

use rstest::rstest;

use super::{
    BYOK_RESOURCE_METADATA_PATH, TOKEN_NO_CACHE_HEADERS, byok_www_authenticate,
    passthrough_metadata_url, passthrough_www_authenticate, route_relative_request_path,
    well_known_root_suffix,
};

fn optional(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

#[rstest]
#[case::unset("", "")]
#[case::slash("/", "")]
#[case::prefixed("/litellm", "/litellm")]
#[case::nested("/litellm/mcp/", "/litellm/mcp/")]
fn root_suffix_matches_litellm(#[case] root: &str, #[case] expected: &str) {
    assert_eq!(well_known_root_suffix(root), expected);
}

#[rstest]
#[case::original_path("/litellm/foo/mcp", "/foo/mcp", "", "/litellm", "/foo/mcp")]
#[case::falls_back_to_path("", "/mcp/foo", "", "", "/mcp/foo")]
#[case::empty_original_path("", "/litellm/mcp/foo", "/litellm", "", "/mcp/foo")]
#[case::app_root_wins("/app/foo/mcp", "/foo/mcp", "/app", "/other", "/foo/mcp")]
#[case::trailing_slash_root("/litellm/foo/mcp", "/foo/mcp", "/litellm/", "", "/foo/mcp")]
#[case::no_segment_boundary(
    "/litellmfoo/mcp",
    "/litellmfoo/mcp",
    "",
    "/litellm",
    "/litellmfoo/mcp"
)]
#[case::root_equals_path("/litellm", "/litellm", "", "/litellm", "")]
fn route_relative_request_path_matches_litellm(
    #[case] original_path: &str,
    #[case] path: &str,
    #[case] app_root_path: &str,
    #[case] root_path: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        route_relative_request_path(
            optional(original_path),
            path,
            optional(app_root_path),
            optional(root_path)
        ),
        expected
    );
}

#[rstest]
#[case::server_first(
    "/foo/mcp",
    "foo",
    "",
    "https://base.example.com/.well-known/oauth-protected-resource/foo/mcp"
)]
#[case::aggregate(
    "/mcp/foo",
    "foo",
    "",
    "https://base.example.com/.well-known/oauth-protected-resource/mcp/foo"
)]
#[case::server_first_with_root(
    "/foo/mcp",
    "foo",
    "/litellm",
    "https://base.example.com/.well-known/oauth-protected-resource/litellm/foo/mcp"
)]
#[case::aggregate_with_root(
    "/mcp/foo",
    "foo",
    "/litellm",
    "https://base.example.com/.well-known/oauth-protected-resource/litellm/mcp/foo"
)]
fn metadata_url_uses_the_request_spelling(
    #[case] route_path: &str,
    #[case] server_name: &str,
    #[case] root_suffix: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        passthrough_metadata_url(
            "https://base.example.com",
            route_path,
            server_name,
            root_suffix
        ),
        expected
    );
}

#[rstest]
#[case::plain(false, "Bearer resource_metadata=\"https://base/meta\"")]
#[case::invalid_token(
    true,
    "Bearer error=\"invalid_token\", resource_metadata=\"https://base/meta\""
)]
fn www_authenticate_matches_litellm(#[case] invalid_token: bool, #[case] expected: &str) {
    assert_eq!(
        passthrough_www_authenticate("https://base/meta", invalid_token),
        expected
    );
}

#[rstest]
#[case::unset_proxy("", "/litellm", "/litellm")]
#[case::unset_proxy_trims_root("", "/litellm//", "/litellm")]
#[case::configured_proxy("https://p.example.com/", "/litellm", "https://p.example.com")]
fn byok_www_authenticate_matches_litellm(
    #[case] proxy_base_url: &str,
    #[case] request_root_path: &str,
    #[case] expected_base: &str,
) {
    assert_eq!(
        byok_www_authenticate(proxy_base_url, request_root_path),
        format!("Bearer resource_metadata=\"{expected_base}{BYOK_RESOURCE_METADATA_PATH}\"")
    );
}

#[test]
fn token_no_cache_headers_are_litellms() {
    assert_eq!(
        TOKEN_NO_CACHE_HEADERS,
        [("Cache-Control", "no-store"), ("Pragma", "no-cache")]
    );
}
