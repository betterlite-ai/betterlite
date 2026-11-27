#![cfg(test)]

use rstest::rstest;

use super::RedirectAllowlists;
use super::loopback::validate_loopback_redirect_uri;
use super::origins::parse_trusted_redirect_origins;
use super::trusted::{matches_trusted_origin_entry, validate_trusted_redirect_uri};
use crate::oauth_resource::base_url::RequestOrigin;

const NON_LOOPBACK_HINT: &str =
    "Native MCP clients should register a callback on http://127.0.0.1:<port>/...";
const DEFAULT_CALLBACK: &str = "cursor://anysphere.cursor-mcp/oauth/callback";

#[rstest]
#[case::loopback_native_callback("http://127.0.0.1:1456/callback", None)]
#[case::loopback_ipv4_loopback("http://127.0.0.2:8080/callback", None)]
#[case::loopback_ipv6_loopback("http://[0:0:0:0:0:0:0:1]:1456/callback", None)]
#[case::loopback_localhost("http://localhost:1456/callback", None)]
#[case::loopback_non_loopback_rejected(
    "https://evil.example/callback",
    Some(("redirect_uri must use a loopback host (localhost or 127.0.0.0/8).", Some(NON_LOOPBACK_HINT)))
)]
#[case::loopback_fragment_rejected(
    "http://127.0.0.1:1456/callback#frag",
    Some(("redirect_uri must not contain a URL fragment (#...).", None))
)]
#[case::loopback_no_scheme(
    "127.0.0.1:1456/callback",
    Some(("redirect_uri scheme '' is not allowed; use http or https.", None))
)]
fn redirect_loopback_policy(
    #[case] redirect_uri: &str,
    #[case] rejection: Option<(&str, Option<&str>)>,
) {
    let rejected = validate_loopback_redirect_uri(redirect_uri)
        .err()
        .map(|error| (error.description, error.hint));
    let expected =
        rejection.map(|(description, hint)| (description.to_owned(), hint.map(str::to_owned)));
    assert_eq!(rejected, expected);
}

#[derive(Clone, Copy, Default)]
struct TrustedCase {
    base_url: &'static str,
    redirect_uri: &'static str,
    proxy_base_url: &'static str,
    forwarded: Option<(&'static str, &'static str, &'static str)>,
    from_trusted_proxy: bool,
    origins: &'static [&'static str],
    native_callbacks: &'static [&'static str],
    rejection: Option<&'static str>,
}

fn trusted_outcome(case: TrustedCase) -> Result<(), String> {
    let (proto, host, port) = match case.forwarded {
        Some((proto, host, port)) => (Some(proto), Some(host), Some(port)),
        None => (None, None, None),
    };
    let origin = RequestOrigin {
        base_url: case.base_url,
        x_forwarded_proto: proto,
        x_forwarded_host: host,
        x_forwarded_port: port,
        from_trusted_proxy: case.from_trusted_proxy,
    };
    let allowlists = RedirectAllowlists {
        origins: case
            .origins
            .iter()
            .map(|entry| (*entry).to_owned())
            .collect(),
        native_callbacks: case
            .native_callbacks
            .iter()
            .map(|entry| (*entry).to_owned())
            .collect(),
    };
    validate_trusted_redirect_uri(&origin, case.proxy_base_url, case.redirect_uri, &allowlists)
        .map_err(|rejected| rejected.description)
}

#[rstest]
#[case::same_origin_proxy(TrustedCase {
    base_url: "https://proxy.example.com",
    redirect_uri: "https://proxy.example.com/ui/mcp/oauth/callback",
    ..TrustedCase::default()
})]
#[case::trusted_https_allowlisted_origin(TrustedCase {
    base_url: "http://proxy.internal:4000",
    redirect_uri: "https://app.example.com/callback",
    origins: &["app.example.com"],
    ..TrustedCase::default()
})]
#[case::trusted_loopback_fallback(TrustedCase {
    base_url: "https://proxy.example.com",
    redirect_uri: "http://127.0.0.2:1456/callback",
    ..TrustedCase::default()
})]
#[case::trusted_same_origin_normalized_port(TrustedCase {
    base_url: "http://internal.example:4000",
    redirect_uri: "https://proxy.example.com/callback",
    forwarded: Some(("https", "proxy.example.com", "443")),
    from_trusted_proxy: true,
    ..TrustedCase::default()
})]
#[case::trusted_native_callback_accepted_first(TrustedCase {
    base_url: "http://proxy.internal:4000",
    redirect_uri: DEFAULT_CALLBACK,
    native_callbacks: &[DEFAULT_CALLBACK],
    ..TrustedCase::default()
})]
#[case::trusted_shape_rejection_passed_through(TrustedCase {
    base_url: "https://proxy.example.com",
    redirect_uri: "https://proxy.example.com/callback#frag",
    rejection: Some("redirect_uri must not contain a URL fragment (#...)."),
    ..TrustedCase::default()
})]
#[case::trusted_proxy_base_url_overrides_header(TrustedCase {
    base_url: "http://internal.example:4000",
    redirect_uri: "https://public.example.com/callback",
    proxy_base_url: "https://public.example.com",
    forwarded: Some(("https", "wrong.example.com", "443")),
    from_trusted_proxy: true,
    ..TrustedCase::default()
})]
fn redirect_trust_policy(#[case] case: TrustedCase) {
    let rejection = trusted_outcome(case).err();
    assert_eq!(rejection, case.rejection.map(str::to_owned));
}

#[test]
fn trusted_wildcard_subdomain() {
    let origin = RequestOrigin {
        base_url: "http://proxy.internal:4000",
        x_forwarded_proto: None,
        x_forwarded_host: None,
        x_forwarded_port: None,
        from_trusted_proxy: false,
    };
    let allowlists = RedirectAllowlists {
        origins: parse_trusted_redirect_origins("*.Example.com"),
        native_callbacks: Vec::new(),
    };
    assert_eq!(
        validate_trusted_redirect_uri(&origin, "", "https://app.example.com/cb", &allowlists),
        Ok(())
    );
    let refused = validate_trusted_redirect_uri(&origin, "", "https://example.com/cb", &allowlists)
        .unwrap_err();
    assert_eq!(
        refused.redirect_uri.as_deref(),
        Some("https://example.com/cb")
    );
}

#[test]
fn trusted_untrusted_proxy_ignores_headers() {
    let untrusted = TrustedCase {
        base_url: "http://internal.example:4000",
        redirect_uri: "https://proxy.example.com/callback",
        forwarded: Some(("https", "proxy.example.com", "443")),
        ..TrustedCase::default()
    };
    let rejection = trusted_outcome(untrusted).unwrap_err();
    assert_eq!(
        rejection,
        "redirect_uri origin (https://proxy.example.com) does not match the proxy origin. scheme: redirect_uri uses 'https', but the proxy resolved a different scheme (TLS often terminates at ingress \u{2014} set PROXY_BASE_URL to https://\u{2026} or trust X-Forwarded-Proto from your ingress); host/port: redirect_uri 'proxy.example.com' does not match the proxy origin"
    );
    let trusted = TrustedCase {
        from_trusted_proxy: true,
        ..untrusted
    };
    assert_eq!(trusted_outcome(trusted), Ok(()));
}

#[test]
fn trusted_allowlists_come_from_argument() {
    let origin = RequestOrigin {
        base_url: "https://proxy.example.com",
        x_forwarded_proto: None,
        x_forwarded_host: None,
        x_forwarded_port: None,
        from_trusted_proxy: false,
    };
    let allowlists = RedirectAllowlists {
        origins: parse_trusted_redirect_origins("app.example.com"),
        native_callbacks: Vec::new(),
    };
    assert_eq!(
        validate_trusted_redirect_uri(&origin, "", "https://app.example.com/callback", &allowlists),
        Ok(())
    );
    let insecure =
        validate_trusted_redirect_uri(&origin, "", "http://app.example.com/callback", &allowlists)
            .unwrap_err();
    assert_eq!(
        insecure.redirect_uri.as_deref(),
        Some("http://app.example.com/callback")
    );
    let refused = validate_trusted_redirect_uri(
        &origin,
        "",
        "https://app.example.com/callback",
        &RedirectAllowlists::default(),
    )
    .unwrap_err();
    assert_eq!(
        refused.redirect_uri.as_deref(),
        Some("https://app.example.com/callback")
    );
}

#[test]
fn redirect_trust_rejections_name_the_origin_or_the_allowlist() {
    let origin = RequestOrigin {
        base_url: "https://proxy.example.com",
        x_forwarded_proto: None,
        x_forwarded_host: None,
        x_forwarded_port: None,
        from_trusted_proxy: false,
    };
    let known = validate_trusted_redirect_uri(
        &origin,
        "",
        "https://evil.example/callback",
        &RedirectAllowlists::default(),
    )
    .unwrap_err();
    assert_eq!(
        known.description,
        "redirect_uri origin (https://evil.example) does not match the proxy origin. host/port: redirect_uri 'evil.example' does not match the proxy origin"
    );
    assert_eq!(known.hint.as_deref(), Some(TRUSTED_REDIRECT_HINT_FOR_TESTS));
    let unknown = RequestOrigin {
        base_url: "http://[::1",
        ..origin
    };
    let unresolved = validate_trusted_redirect_uri(
        &unknown,
        "",
        "https://evil.example/callback",
        &RedirectAllowlists::default(),
    )
    .unwrap_err();
    assert_eq!(
        unresolved.description,
        "redirect_uri ('https://evil.example/callback') is not allowed: not same-origin with the proxy origin, not loopback, and not listed in MCP_TRUSTED_REDIRECT_ORIGINS."
    );
}

const TRUSTED_REDIRECT_HINT_FOR_TESTS: &str = "Align the proxy public URL with the browser URL. Set PROXY_BASE_URL to your HTTPS origin (e.g. https://litellm.example.com), or enable general_settings.use_x_forwarded_for with mcp_trusted_proxy_ranges for your ingress. If the redirect_uri is a legitimate separate-origin OAuth client (e.g. a web app registering with the proxy from another host via dynamic client registration), add its origin to MCP_TRUSTED_REDIRECT_ORIGINS. Verify: curl https://<host>/.well-known/oauth-authorization-server | jq .issuer \u{2014} issuer must match window.location.origin in the UI.";

#[test]
fn origins_empty_text() {
    assert_eq!(parse_trusted_redirect_origins(""), Vec::<String>::new());
    assert_eq!(parse_trusted_redirect_origins("   "), Vec::<String>::new());
    assert_eq!(parse_trusted_redirect_origins(","), Vec::<String>::new());
    assert_eq!(
        parse_trusted_redirect_origins("https://:443"),
        Vec::<String>::new()
    );
}

#[rstest]
#[case::origins_strip_scheme_path_and_case(
    " https://Shop.Example.com:443/path , x.example.com:8443/a ",
    &["shop.example.com", "x.example.com:8443"]
)]
#[case::origins_strip_default_https_port("App.Example.com:443", &["app.example.com"])]
#[case::origins_keep_wildcard_port("*.Example.com:443", &["*.example.com:443"])]
fn redirect_origins(#[case] text: &str, #[case] expected: &[&str]) {
    let parsed = parse_trusted_redirect_origins(text);
    let expected: Vec<String> = expected.iter().map(|entry| (*entry).to_owned()).collect();
    assert_eq!(parsed, expected);
}

#[rstest]
#[case::exact("app.example.com", "app.example.com", true)]
#[case::exact_with_port("app.example.com:8443", "app.example.com:8443", true)]
#[case::port_mismatch("app.example.com:8443", "app.example.com", false)]
#[case::wildcard_child("app.example.com", "*.example.com", true)]
#[case::wildcard_deep_child("a.b.example.com", "*.example.com", true)]
#[case::wildcard_apex("example.com", "*.example.com", false)]
#[case::wildcard_neighbour("notexample.com", "*.example.com", false)]
#[case::wildcard_empty_suffix("app.example.com", "*.", false)]
#[case::wildcard_dotted_suffix("app.example.com", "*..example.com", false)]
#[case::wildcard_ignores_port("app.example.com:8443", "*.example.com", true)]
#[case::wildcard_empty_suffix_trailing_dot("example.com.", "*.", false)]
#[case::wildcard_dotted_suffix_host("app..example.com", "*..example.com", false)]
fn trusted_origin_entries_match_hosts_and_wildcards(
    #[case] netloc: &str,
    #[case] entry: &str,
    #[case] expected: bool,
) {
    assert_eq!(matches_trusted_origin_entry(netloc, entry), expected);
}
