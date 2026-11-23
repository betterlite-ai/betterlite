#![cfg(test)]

use http::StatusCode;
use rstest::rstest;

use super::{
    DEFAULT_NATIVE_REDIRECT_URIS, INVALID_REQUEST, RedirectShape, RedirectUriRejected,
    is_loopback_redirect_host, native_callbacks, parse_redirect_uri, validate_redirect_uri_shape,
};

const BUILTIN: &str = "cursor://anysphere.cursor-mcp/oauth/callback";
const SCHEME_HINT: &str =
    "Add the full URI to MCP_TRUSTED_NATIVE_REDIRECT_URIS for custom native clients.";

#[test]
fn native_callbacks_holds_only_the_builtin_cursor_callback_for_empty_text() {
    let builtin = vec![BUILTIN.to_owned()];
    assert_eq!(native_callbacks(""), builtin);
    assert_eq!(native_callbacks("  "), builtin);
    assert_eq!(native_callbacks(","), builtin);
    assert_eq!(DEFAULT_NATIVE_REDIRECT_URIS, [BUILTIN]);
}

#[rstest]
#[case::trims_and_lowercases(
    "  MyApp://Host/OAuth/* , Second://x/y* ",
    vec![BUILTIN.to_owned(), "myapp://host/oauth/*".to_owned(), "second://x/y*".to_owned()]
)]
#[case::skips_empty_entries(
    ", ,myapp://host/cb, ,",
    vec![BUILTIN.to_owned(), "myapp://host/cb".to_owned()]
)]
#[case::skips_repeats_of_the_builtin_and_of_an_earlier_entry(
    "CURSOR://anysphere.cursor-mcp/oauth/callback,myapp://host/cb, MYAPP://host/cb ,myapp://host/cb",
    vec![BUILTIN.to_owned(), "myapp://host/cb".to_owned()]
)]
fn native_callbacks_trims_lowercases_and_dedupes(
    #[case] configured: &str,
    #[case] expected: Vec<String>,
) {
    assert_eq!(native_callbacks(configured), expected);
}

#[rstest]
#[case::builtin(BUILTIN, "")]
#[case::builtin_in_any_letter_case("CURSOR://AnySphere.Cursor-MCP/OAuth/Callback", "")]
#[case::configured_exact("myapp://host/cb", "myapp://host/cb")]
#[case::configured_exact_in_any_letter_case("MYAPP://HOST/CB", "myapp://host/cb")]
#[case::configured_entry_any_letter_case("myapp://host/cb", "MyApp://Host/CB")]
fn exact_native_callbacks_are_accepted(#[case] uri: &str, #[case] configured: &str) {
    let parsed = parse_redirect_uri(uri).unwrap();
    assert_eq!(
        validate_redirect_uri_shape(&parsed, &native_callbacks(configured)).unwrap(),
        RedirectShape::NativeCallback
    );
}

#[rstest]
#[case::star_takes_the_bare_prefix("myapp://host/cb", "myapp://host/cb*")]
#[case::star_takes_a_trailing_slash("myapp://host/cb/", "myapp://host/cb*")]
#[case::star_takes_a_deeper_path("myapp://host/cb/x", "myapp://host/cb*")]
#[case::slash_prefix_takes_any_suffix("myapp://host/cb-2", "myapp://host/cb/*")]
#[case::slash_prefix_takes_an_empty_suffix("myapp://host/oauth/", "myapp://host/oauth/*")]
fn wildcard_native_callbacks_take_deeper_paths(#[case] uri: &str, #[case] configured: &str) {
    let parsed = parse_redirect_uri(uri).unwrap();
    assert_eq!(
        validate_redirect_uri_shape(&parsed, &native_callbacks(configured)).unwrap(),
        RedirectShape::NativeCallback
    );
}

#[rstest]
#[case::star_refuses_a_sibling("myapp://host/cb-2", "myapp://host/cb*")]
#[case::slash_prefix_refuses_a_missing_slash("myapp://host/oauth", "myapp://host/oauth/*")]
#[case::slash_prefix_refuses_a_sibling("myapp://host/oauthx", "myapp://host/oauth/*")]
#[case::exact_refuses_a_deeper_path("myapp://host/cb/x", "myapp://host/cb")]
fn unlisted_native_callbacks_are_refused(#[case] uri: &str, #[case] configured: &str) {
    let parsed = parse_redirect_uri(uri).unwrap();
    let rejected = validate_redirect_uri_shape(&parsed, &native_callbacks(configured)).unwrap_err();
    assert_eq!(
        rejected.description,
        "redirect_uri scheme 'myapp' is not allowed; use http/https or a registered native callback (e.g. cursor://)."
    );
    assert_eq!(rejected.hint.as_deref(), Some(SCHEME_HINT));
    assert_eq!(rejected.redirect_uri, None);
}

// the native match refuses these before comparing, so an entry equal to the URI does not help
#[rstest]
#[case::fragment("myapp://host/cb#frag", "myapp://host/cb", "myapp")]
#[case::query("myapp://host/cb?x=1", "myapp://host/cb", "myapp")]
#[case::userinfo("myapp://user:pass@host/cb", "myapp://user:pass@host/cb", "myapp")]
#[case::empty_netloc("mailto:user@example.com", "mailto:user@example.com", "mailto")]
fn an_allowlisted_entry_does_not_excuse_a_fragment_query_userinfo_or_no_host(
    #[case] uri: &str,
    #[case] configured: &str,
    #[case] scheme: &str,
) {
    let parsed = parse_redirect_uri(uri).unwrap();
    let rejected = validate_redirect_uri_shape(&parsed, &native_callbacks(configured)).unwrap_err();
    assert_eq!(
        rejected.description,
        format!(
            "redirect_uri scheme '{scheme}' is not allowed; use http/https or a registered native callback (e.g. cursor://)."
        )
    );
    assert_eq!(rejected.hint.as_deref(), Some(SCHEME_HINT));
    assert_eq!(rejected.redirect_uri, None);
}

#[rstest]
#[case::custom_scheme("myapp://host/cb", "myapp")]
#[case::scheme_with_a_fragment("ftp://host/cb#x", "ftp")]
#[case::builtin_with_a_query("cursor://anysphere.cursor-mcp/oauth/callback?x=1", "cursor")]
#[case::builtin_with_a_fragment("cursor://anysphere.cursor-mcp/oauth/callback#frag", "cursor")]
#[case::builtin_with_a_nonempty_fragment(
    "cursor://anysphere.cursor-mcp/oauth/callback?x=1#",
    "cursor"
)]
#[case::builtin_with_userinfo("cursor://user:pass@anysphere.cursor-mcp/oauth/callback", "cursor")]
#[case::builtin_with_empty_userinfo("cursor://@anysphere.cursor-mcp/oauth/callback", "cursor")]
#[case::builtin_with_a_backslash("cursor://anysphere.cursor-mcp\\/oauth/callback", "cursor")]
#[case::builtin_with_a_deeper_path("cursor://anysphere.cursor-mcp/oauth/callback/extra", "cursor")]
#[case::no_scheme("not a url", "")]
#[case::mailto("mailto:user@example.com", "mailto")]
fn schemes_that_are_not_http_or_native_are_refused(#[case] uri: &str, #[case] scheme: &str) {
    let parsed = parse_redirect_uri(uri).unwrap();
    let rejected = validate_redirect_uri_shape(&parsed, &native_callbacks("")).unwrap_err();
    assert_eq!(
        rejected.description,
        format!(
            "redirect_uri scheme '{scheme}' is not allowed; use http/https or a registered native callback (e.g. cursor://)."
        )
    );
    assert_eq!(rejected.hint.as_deref(), Some(SCHEME_HINT));
    assert_eq!(rejected.redirect_uri, None);
}

#[rstest]
#[case::https("https://example.com/cb")]
#[case::http("http://example.com/cb")]
#[case::uppercase_scheme("HTTP://Host/CB")]
#[case::empty_fragment("https://host/cb#")]
#[case::query("https://host/cb?x=1")]
fn web_uris_need_the_callers_trust_policy(#[case] uri: &str) {
    let parsed = parse_redirect_uri(uri).unwrap();
    assert_eq!(
        validate_redirect_uri_shape(&parsed, &native_callbacks("")).unwrap(),
        RedirectShape::NeedsTrustPolicy
    );
}

#[rstest]
#[case::fragment(
    "https://host/cb#frag",
    "redirect_uri must not contain a URL fragment (#...)."
)]
#[case::fragment_before_backslash(
    "https://host\\evil/cb#frag",
    "redirect_uri must not contain a URL fragment (#...)."
)]
#[case::no_host(
    "https:///cb",
    "redirect_uri must include a host (e.g. https://your-host/path)."
)]
#[case::empty_netloc(
    "http://",
    "redirect_uri must include a host (e.g. https://your-host/path)."
)]
#[case::userinfo(
    "https://user:pass@host/cb",
    "redirect_uri must not contain userinfo (user:pass@host)."
)]
#[case::empty_userinfo(
    "https://@host/cb",
    "redirect_uri must not contain userinfo (user:pass@host)."
)]
#[case::backslash(
    "https://host\\evil/cb",
    "redirect_uri host must not contain backslashes."
)]
fn web_uris_with_a_hygiene_problem_are_refused(#[case] uri: &str, #[case] description: &str) {
    let parsed = parse_redirect_uri(uri).unwrap();
    let rejected = validate_redirect_uri_shape(&parsed, &native_callbacks("")).unwrap_err();
    assert_eq!(rejected.description, description);
    assert_eq!(rejected.hint, None);
    assert_eq!(rejected.redirect_uri, None);
}

#[rstest]
#[case::localhost("http://localhost/cb", true)]
#[case::localhost_in_any_letter_case("http://LOCALHOST/cb", true)]
#[case::loopback_v4("http://127.0.0.1/cb", true)]
#[case::loopback_v4_end_of_range("http://127.255.255.254/cb", true)]
#[case::loopback_v4_with_port("http://127.0.0.1:8080/cb", true)]
#[case::loopback_v6("http://[::1]/cb", true)]
#[case::loopback_v6_full_form("http://[0:0:0:0:0:0:0:1]/cb", true)]
#[case::loopback_v6_with_port("http://[::1]:8080/cb", true)]
#[case::ipv4_mapped_loopback("http://[::ffff:127.0.0.1]/cb", true)]
#[case::ipv4_mapped_loopback_in_range("http://[::ffff:127.0.0.2]/cb", true)]
#[case::unspecified_v4("http://0.0.0.0/cb", false)]
#[case::unspecified_v6("http://[::]/cb", false)]
#[case::ipv4_mapped_public("http://[::ffff:8.8.8.8]/cb", false)]
#[case::name_that_ends_in_localhost("http://localhost.evil.example/cb", false)]
#[case::ipv4_with_leading_zeros("http://127.000.000.001/cb", false)]
#[case::trailing_dot("http://127.0.0.1./cb", false)]
#[case::no_host("http:///cb", false)]
fn loopback_hosts_are_recognised(#[case] uri: &str, #[case] expected: bool) {
    let parsed = parse_redirect_uri(uri).unwrap();
    assert_eq!(is_loopback_redirect_host(&parsed), expected);
}

#[rstest]
#[case::unclosed_bracket("http://[::1/cb")]
#[case::ipv4_in_brackets("http://[127.0.0.1]/cb")]
#[case::not_an_address("http://[gg::1]/cb")]
#[case::ipvfuture_without_address("http://[v1.]/cb")]
fn unparseable_uris_are_refused(#[case] uri: &str) {
    let rejected = parse_redirect_uri(uri).unwrap_err();
    assert_eq!(rejected.description, "redirect_uri is not a valid URL.");
    assert_eq!(
        rejected.hint.as_deref(),
        Some(
            "Use a full absolute URL for redirect_uri (e.g. https://your-host/ui/mcp/oauth/callback)."
        )
    );
    assert_eq!(rejected.redirect_uri, None);
}

#[test]
fn the_rejection_details_are_a_400_invalid_request_with_only_the_set_optionals() {
    assert_eq!(RedirectUriRejected::STATUS, StatusCode::BAD_REQUEST);
    assert_eq!(INVALID_REQUEST, "invalid_request");
    let bare = RedirectUriRejected {
        description: "redirect_uri must include a host (e.g. https://your-host/path).".to_owned(),
        hint: None,
        redirect_uri: None,
    };
    assert_eq!(
        serde_json::to_string(&bare.detail()).unwrap(),
        r#"{"error":"invalid_request","error_description":"redirect_uri must include a host (e.g. https://your-host/path)."}"#
    );
    let full = RedirectUriRejected {
        description: "denied".to_owned(),
        hint: Some("register the callback".to_owned()),
        redirect_uri: Some("https://host/cb".to_owned()),
    };
    assert_eq!(
        serde_json::to_string(&full.detail()).unwrap(),
        r#"{"error":"invalid_request","error_description":"denied","hint":"register the callback","redirect_uri":"https://host/cb"}"#
    );
    assert_eq!(full.to_string(), "denied");
}
