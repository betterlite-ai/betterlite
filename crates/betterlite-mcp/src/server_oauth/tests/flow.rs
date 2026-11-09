fn secret() -> SecretString {
    SecretString::from("secret".to_owned())
}

fn m2m_server() -> MCPServer {
    server_from_json(
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
    )
}

#[rstest]
#[case::client_credentials(Some("client_credentials"), Some(OAuth2Flow::ClientCredentials))]
#[case::authorization_code(Some("authorization_code"), Some(OAuth2Flow::AuthorizationCode))]
#[case::unknown(Some("implicit"), None)]
#[case::absent(None, None)]
fn flow_explicit_maps_only_the_two_flows(
    #[case] text: Option<&str>,
    #[case] expected: Option<OAuth2Flow>,
) {
    assert_eq!(explicit_oauth2_flow(text), expected);
}

#[test]
fn flow_resolve_returns_an_explicit_flow_before_inferring() {
    let secret = secret();
    let inputs = FlowResolution {
        auth_type: Some(MCPAuth::ApiKey),
        oauth2_flow: Some(OAuth2Flow::AuthorizationCode),
        token_url: Some("https://idp.test/token"),
        authorization_url: Some("https://idp.test/authorize"),
        client_id: Some("client"),
        client_secret: Some(&secret),
    };
    assert_eq!(
        resolve_oauth2_flow(inputs),
        Some(OAuth2Flow::AuthorizationCode)
    );
}

#[test]
fn flow_resolve_infers_client_credentials_from_the_m2m_shape() {
    let secret = secret();
    let inputs = FlowResolution {
        auth_type: Some(MCPAuth::Oauth2),
        oauth2_flow: None,
        token_url: Some("https://idp.test/token"),
        authorization_url: None,
        client_id: Some("client"),
        client_secret: Some(&secret),
    };
    assert_eq!(
        resolve_oauth2_flow(inputs),
        Some(OAuth2Flow::ClientCredentials)
    );
}

#[rstest]
#[case::missing_token_url(None, Some("client"), true)]
#[case::missing_client_id(Some("https://idp.test/token"), None, true)]
#[case::missing_secret(Some("https://idp.test/token"), Some("client"), false)]
fn flow_resolve_needs_every_m2m_field(
    #[case] token_url: Option<&str>,
    #[case] client_id: Option<&str>,
    #[case] with_secret: bool,
) {
    let secret = secret();
    let inputs = FlowResolution {
        auth_type: Some(MCPAuth::Oauth2),
        oauth2_flow: None,
        token_url,
        authorization_url: None,
        client_id,
        client_secret: with_secret.then_some(&secret),
    };
    assert_eq!(resolve_oauth2_flow(inputs), None);
}

#[test]
fn flow_resolve_rejects_other_auth_types_and_a_pinned_authorize_url() {
    let secret = secret();
    let wrong_auth = FlowResolution {
        auth_type: Some(MCPAuth::ApiKey),
        oauth2_flow: None,
        token_url: Some("https://idp.test/token"),
        authorization_url: None,
        client_id: Some("client"),
        client_secret: Some(&secret),
    };
    assert_eq!(resolve_oauth2_flow(wrong_auth), None);
    let pinned_authorize = FlowResolution {
        auth_type: Some(MCPAuth::Oauth2),
        oauth2_flow: None,
        token_url: Some("https://idp.test/token"),
        authorization_url: Some("https://idp.test/authorize"),
        client_id: Some("client"),
        client_secret: Some(&secret),
    };
    assert_eq!(resolve_oauth2_flow(pinned_authorize), None);
}

#[test]
fn flow_effective_prefers_the_stored_flow_over_the_shape() {
    let server = server_from_json(
        r#"{
            "server_id": "test-stamped",
            "name": "test-stamped",
            "url": "https://mcp.test/mcp",
            "transport": "http",
            "auth_type": "oauth2",
            "oauth2_flow": "authorization_code",
            "token_url": "https://idp.test/token",
            "client_id": "client",
            "client_secret": "secret"
        }"#,
    );
    assert_eq!(
        effective_oauth2_flow(&server),
        Some(OAuth2Flow::AuthorizationCode)
    );
}

#[test]
fn flow_effective_infers_client_credentials_for_an_unstamped_m2m_row() {
    assert_eq!(
        effective_oauth2_flow(&m2m_server()),
        Some(OAuth2Flow::ClientCredentials)
    );
    let interactive = oauth_server();
    assert_eq!(effective_oauth2_flow(&interactive), None);
}

#[test]
fn flow_request_borrows_the_server_when_the_flow_is_already_known() {
    let stamped = server_from_json(
        r#"{
            "server_id": "test-stamped",
            "name": "test-stamped",
            "url": "https://mcp.test/mcp",
            "transport": "http",
            "auth_type": "oauth2",
            "oauth2_flow": "authorization_code"
        }"#,
    );
    assert!(matches!(flow_for_request(&stamped), Cow::Borrowed(_)));
    let interactive = oauth_server();
    assert!(matches!(flow_for_request(&interactive), Cow::Borrowed(_)));
}

#[test]
fn flow_request_stamps_the_inferred_flow_and_warns() {
    let server = m2m_server();
    let (captured, _guard) = capture();
    let resolved = flow_for_request(&server);
    assert!(matches!(&resolved, Cow::Owned(_)));
    assert_eq!(
        resolved.fields.oauth2_flow,
        Some(OAuth2Flow::ClientCredentials)
    );
    let text = captured.text();
    assert!(text.contains("test-m2m"));
    assert!(text.contains("client_credentials"));
    assert!(text.contains("oauth2_flow explicitly"));
}
