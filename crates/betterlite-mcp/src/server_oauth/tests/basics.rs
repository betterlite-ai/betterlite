#[rstest]
#[case::absent(None, None)]
#[case::empty(Some(""), None)]
#[case::whitespace(Some("   "), None)]
#[case::padded(Some("  https://idp.test/token  "), Some("https://idp.test/token"))]
fn basics_blank_to_none_strips_and_drops_empty(
    #[case] value: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(blank_to_none(value), expected);
}

#[rstest]
#[case::issuer_and_discovery(Some("https://idp.test"), true, true)]
#[case::issuer_without_discovery(Some("https://idp.test"), false, false)]
#[case::no_issuer(None, true, false)]
#[case::empty_issuer(Some(""), true, false)]
#[case::blank_issuer(Some("   "), true, false)]
fn basics_uses_issuer_anchor_needs_an_issuer_on_a_discovery_type(
    #[case] issuer: Option<&str>,
    #[case] is_discovery_auth_type: bool,
    #[case] expected: bool,
) {
    assert_eq!(uses_issuer_anchor(issuer, is_discovery_auth_type), expected);
}

#[rstest]
#[case::url(Some("https://mcp.test/mcp"), false, true)]
#[case::empty_url(Some(""), false, false)]
#[case::no_url(None, false, false)]
#[case::anchored_without_url(None, true, true)]
fn basics_has_discovery_source_needs_a_url_or_an_anchor(
    #[case] server_url: Option<&str>,
    #[case] use_issuer_anchor: bool,
    #[case] expected: bool,
) {
    assert_eq!(
        has_discovery_source(server_url, use_issuer_anchor),
        expected
    );
}

#[test]
fn basics_discovery_auth_types_list_oauth2_and_the_two_forwarded_modes() {
    assert_eq!(
        UPSTREAM_DISCOVERY_AUTH_TYPES,
        [
            MCPAuth::Oauth2,
            MCPAuth::TruePassthrough,
            MCPAuth::OauthDelegate
        ]
    );
}

#[rstest]
#[case::both_absent(None, None, true)]
#[case::configured_exchange(Some("https://idp.test/exchange"), None, false)]
#[case::configured_token(None, Some("https://idp.test/token"), false)]
#[case::empty_exchange(Some(""), None, true)]
fn basics_obo_needs_discovery_without_any_configured_endpoint(
    #[case] token_exchange_endpoint: Option<&str>,
    #[case] token_url: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(
        obo_needs_discovery(
            Some(MCPAuth::Oauth2TokenExchange),
            token_exchange_endpoint,
            token_url
        ),
        expected
    );
}

#[test]
fn basics_obo_needs_discovery_ignores_other_auth_types() {
    assert!(!obo_needs_discovery(Some(MCPAuth::Oauth2), None, None));
    assert!(!obo_needs_discovery(None, None, None));
}

#[rstest]
#[case::no_issuer(None, true)]
#[case::non_discovery_type(Some("https://idp.test"), false)]
fn basics_endpoints_yield_to_issuer_returns_the_inputs_without_an_anchor(
    #[case] issuer: Option<&str>,
    #[case] is_discovery_auth_type: bool,
) {
    let endpoints = ManualEndpoints {
        authorization_url: Some("https://idp.test/authorize"),
        token_url: Some("https://idp.test/token"),
        registration_url: Some("https://idp.test/register"),
    };
    assert_eq!(
        endpoints_yield_to_issuer(issuer, is_discovery_auth_type, endpoints, "test-server"),
        endpoints
    );
}

#[test]
fn basics_endpoints_yield_to_issuer_drops_endpoints_and_warns_in_sorted_order() {
    let endpoints = ManualEndpoints {
        authorization_url: Some("https://idp.test/authorize"),
        token_url: Some("https://idp.test/token"),
        registration_url: Some("https://idp.test/register"),
    };
    let (captured, _guard) = capture();
    let resolved =
        endpoints_yield_to_issuer(Some("https://idp.test"), true, endpoints, "test-server");
    assert_eq!(resolved, ManualEndpoints::default());
    let text = captured.text();
    assert!(text.contains("test-server"));
    assert!(text.contains("authorization_url, registration_url, token_url are not used"));
    assert!(text.contains("manually configured endpoints"));
}

#[test]
fn basics_endpoints_yield_to_issuer_uses_the_singular_verb_for_one_field() {
    let endpoints = ManualEndpoints {
        authorization_url: Some("https://idp.test/authorize"),
        token_url: None,
        registration_url: None,
    };
    let (captured, _guard) = capture();
    let resolved =
        endpoints_yield_to_issuer(Some("https://idp.test"), true, endpoints, "test-server");
    assert_eq!(resolved, ManualEndpoints::default());
    assert!(
        captured
            .text()
            .contains("stored authorization_url is not used")
    );
}

#[test]
fn basics_endpoints_yield_to_issuer_is_silent_without_endpoints_to_discard() {
    let (captured, _guard) = capture();
    let resolved = endpoints_yield_to_issuer(
        Some("https://idp.test"),
        true,
        ManualEndpoints::default(),
        "test-server",
    );
    assert_eq!(resolved, ManualEndpoints::default());
    assert_eq!(captured.text(), "");
}

#[rstest]
#[case::case_port_and_slash("https://IDP.test:443/authorize/", "https://idp.test/authorize")]
#[case::query_and_fragment("https://idp.test/authorize?x=1#frag", "https://idp.test/authorize")]
#[case::userinfo("https://user:pass@idp.test/authorize", "https://idp.test/authorize")]
#[case::path_kept(
    "https://idp.test/oauth2/authorize",
    "https://idp.test/oauth2/authorize"
)]
#[case::unparsable("https://[bad", "https://[bad")]
fn basics_normalized_authorize_endpoint_elides_insignificant_parts(
    #[case] url: &str,
    #[case] expected: &str,
) {
    assert_eq!(normalized_authorize_endpoint(url), expected);
}

#[rstest]
#[case::normalized("https://IDP.test:443/authorize/", "https://idp.test/authorize")]
#[case::unparsable_with_credentials(
    "https://client:s3cret@[idp.test]/authorize?api_key=abc",
    "<unparsable url>"
)]
fn basics_authorize_endpoint_for_log_never_prints_an_unparsable_url(
    #[case] url: &str,
    #[case] expected: &str,
) {
    assert_eq!(authorize_endpoint_for_log(url), expected);
}

#[rstest]
#[case::equal_after_normalization("https://IDP.test:443/", true)]
#[case::different_host("https://other.test", false)]
#[case::empty_claim("", false)]
fn basics_issuer_matches_compares_normalized_string_claims(
    #[case] claimed: &str,
    #[case] expected: bool,
) {
    let value = JsonValue::String(claimed.to_owned());
    assert_eq!(issuer_matches(Some(&value), "https://idp.test"), expected);
}

#[rstest]
#[case::non_string(JsonValue::Bool(true))]
#[case::null(JsonValue::Null)]
fn basics_issuer_matches_rejects_a_non_string_claim(#[case] claimed: JsonValue) {
    assert!(!issuer_matches(Some(&claimed), "https://idp.test"));
}

#[test]
fn basics_issuer_matches_rejects_an_absent_claim() {
    assert!(!issuer_matches(None, "https://idp.test"));
}
