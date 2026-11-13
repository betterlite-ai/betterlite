#[rstest]
#[case::both_absent(None, None, true)]
#[case::empty_token_url(None, Some(""), false)]
#[case::configured_exchange(Some("https://idp.test/exchange"), None, false)]
#[case::configured_token(None, Some("https://idp.test/token"), false)]
fn unresolved_flow_endpoints_missing_token_exchange_uses_none(
    #[case] token_exchange_endpoint: Option<&str>,
    #[case] token_url: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(
        flow_endpoints_missing(
            Some(MCPAuth::Oauth2TokenExchange),
            None,
            None,
            token_url,
            token_exchange_endpoint
        ),
        expected
    );
}

#[rstest]
#[case::missing_token_url(None, true)]
#[case::empty_token_url(Some(""), false)]
#[case::configured_token_url(Some("https://idp.test/token"), false)]
fn unresolved_flow_endpoints_missing_client_credentials_needs_a_token_url(
    #[case] token_url: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(
        flow_endpoints_missing(
            Some(MCPAuth::Oauth2),
            Some(OAuth2Flow::ClientCredentials),
            None,
            token_url,
            None
        ),
        expected
    );
}

#[rstest]
#[case::both_missing(None, None, true)]
#[case::authorization_missing(None, Some("https://idp.test/token"), true)]
#[case::token_missing(Some("https://idp.test/authorize"), None, true)]
#[case::both_present(
    Some("https://idp.test/authorize"),
    Some("https://idp.test/token"),
    false
)]
fn unresolved_flow_endpoints_missing_authorization_code_needs_both(
    #[case] authorization_url: Option<&str>,
    #[case] token_url: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(
        flow_endpoints_missing(
            Some(MCPAuth::Oauth2),
            Some(OAuth2Flow::AuthorizationCode),
            authorization_url,
            token_url,
            None
        ),
        expected
    );
}

#[rstest]
#[case::api_key(Some(MCPAuth::ApiKey))]
#[case::absent(None)]
fn unresolved_flow_endpoints_missing_is_false_outside_discovery_types(
    #[case] auth_type: Option<MCPAuth>,
) {
    assert!(!flow_endpoints_missing(auth_type, None, None, None, None));
}

#[test]
fn unresolved_endpoints_unresolved_entra_obo_stays_unresolved_without_scopes() {
    let mut server = oauth_server();
    server.fields.auth_type = Some(MCPAuth::Oauth2TokenExchange);
    server.fields.token_exchange_profile = "entra_obo".to_owned();
    server.fields.token_url = Some("https://idp.test/token".to_owned());
    assert!(endpoints_unresolved(&server));
    server.fields.scopes = Some(Vec::new());
    assert!(endpoints_unresolved(&server));
    server.fields.scopes = Some(vec!["read".to_owned()]);
    assert!(!endpoints_unresolved(&server));
}

#[test]
fn unresolved_endpoints_unresolved_dcr_bridge_needs_registration_when_unregistered() {
    let mut server = oauth_server();
    server.fields.auth_type = Some(MCPAuth::TruePassthrough);
    server.fields.dcr_bridge = Some(true);
    server.fields.authorization_url = Some("https://idp.test/authorize".to_owned());
    server.fields.token_url = Some("https://idp.test/token".to_owned());
    assert!(server.is_dcr_bridge());
    assert!(endpoints_unresolved(&server));
    server.fields.registration_url = Some("https://idp.test/register".to_owned());
    assert!(!endpoints_unresolved(&server));
    server.fields.registration_url = None;
    server.fields.client_id = Some("client".to_owned());
    assert!(!endpoints_unresolved(&server));
}

#[test]
fn unresolved_endpoints_unresolved_reads_the_effective_flow_not_the_stored_column() {
    let server = server_from_json(
        r#"{
            "server_id": "test-m2m",
            "name": "test-m2m",
            "url": "https://mcp.test/mcp",
            "transport": "http",
            "auth_type": "oauth2",
            "token_url": "https://idp.test/token",
            "client_id": "client",
            "client_secret": "secret"
        }"#,
    );
    assert!(!endpoints_unresolved(&server));
    let without_secret = server_from_json(
        r#"{
            "server_id": "test-m2m",
            "name": "test-m2m",
            "url": "https://mcp.test/mcp",
            "transport": "http",
            "auth_type": "oauth2",
            "token_url": "https://idp.test/token",
            "client_id": "client"
        }"#,
    );
    assert!(endpoints_unresolved(&without_secret));
}

#[rstest]
#[case::url_and_unresolved(Some("https://mcp.test/mcp"), false, true)]
#[case::no_source(None, false, false)]
#[case::anchored_without_url(None, true, true)]
fn unresolved_requires_discovery_needs_a_source_and_missing_endpoints(
    #[case] server_url: Option<&str>,
    #[case] use_issuer_anchor: bool,
    #[case] expected: bool,
) {
    let server = oauth_server();
    assert_eq!(
        requires_discovery(server_url, use_issuer_anchor, &server),
        expected
    );
}

#[test]
fn unresolved_requires_discovery_is_false_once_the_endpoints_resolve() {
    let mut server = oauth_server();
    server.fields.authorization_url = Some("https://idp.test/authorize".to_owned());
    server.fields.token_url = Some("https://idp.test/token".to_owned());
    assert!(!requires_discovery(
        Some("https://mcp.test/mcp"),
        false,
        &server
    ));
}
