#[tokio::test]
async fn build_records_the_identity_fields() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "alias": "test_alias", "server_id": "id-1"}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server("id-1").await.unwrap();
    assert_eq!(server.fields.name, "test_alias");
    assert_eq!(server.fields.alias.as_deref(), Some("test_alias"));
    assert_eq!(server.fields.server_name.as_deref(), Some("test_server"));
    assert_eq!(server.fields.url.as_deref(), Some("https://mcp.test"));
    let Json::Object(entries) = mcp_info(&server) else {
        panic!("the mcp_info must serialize as an object");
    };
    assert_eq!(
        entries.get("server_name"),
        Some(&Json::Text(String::from("test_server")))
    );
}

#[tokio::test]
async fn build_the_first_mapped_alias_wins() {
    let fixture = Fixture::new();
    let mapped = aliases(r#"{"a1": "test_server", "a2": "test_server"}"#);
    let (captured, _guard) = capture();
    fixture
        .load_with(
            &config(r#"{"test_server": {"url": "https://mcp.test"}}"#),
            Some(&mapped),
        )
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.name, "a1");
    assert_eq!(server.fields.alias.as_deref(), Some("a1"));
    assert!(
        captured
            .text()
            .contains("Mapped alias 'a1' to server 'test_server'")
    );
    assert!(!captured.text().contains("Mapped alias 'a2'"));
}

#[tokio::test]
async fn build_an_entry_alias_blocks_the_mapping() {
    let fixture = Fixture::new();
    let mapped = aliases(r#"{"mapped": "test_server"}"#);
    fixture
        .load_with(
            &config(r#"{"test_server": {"url": "https://mcp.test", "alias": "own"}}"#),
            Some(&mapped),
        )
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.name, "own");
    assert_eq!(server.fields.alias.as_deref(), Some("own"));
}

#[tokio::test]
async fn build_metadata_resolves_the_oauth_endpoints() {
    let fixture = Fixture::with_metadata();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "auth_type": "oauth2", "oauth2_flow": "authorization_code"}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.issuer.as_deref(), Some("https://as.test"));
    assert!(!server.fields.issuer_is_anchored);
    assert_eq!(
        server.fields.authorization_url.as_deref(),
        Some("https://as.test/authorize")
    );
    assert_eq!(
        server.fields.token_url.as_deref(),
        Some("https://as.test/token")
    );
    assert_eq!(
        server.fields.registration_url.as_deref(),
        Some("https://as.test/register")
    );
    assert_eq!(
        server.fields.scopes,
        Some(vec![String::from("read"), String::from("write")])
    );
    assert_eq!(server.fields.configured_scopes, None);
    assert_eq!(
        server.fields.oauth2_flow,
        Some(OAuth2Flow::AuthorizationCode)
    );
    let calls = fixture.fetcher.calls();
    assert!(calls.contains(&String::from(
        "https://mcp.test/.well-known/oauth-protected-resource"
    )));
    assert!(calls.contains(&String::from(
        "https://as.test/.well-known/oauth-authorization-server"
    )));
}

#[tokio::test]
async fn build_configured_scopes_win_over_the_discovered_ones() {
    let fixture = Fixture::with_metadata();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "auth_type": "oauth2", "oauth2_flow": "authorization_code", "scopes": "admin"}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.scopes, Some(vec![String::from("admin")]));
    assert_eq!(
        server.fields.configured_scopes,
        Some(vec![String::from("admin")])
    );
}

#[tokio::test]
async fn build_discovery_is_deferred_when_it_is_not_on_startup() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "auth_type": "oauth2", "oauth2_flow": "authorization_code"}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.authorization_url, None);
    assert_eq!(server.fields.token_url, None);
    assert!(fixture.fetcher.calls().is_empty());
    assert!(fixture.discovery.slot_exists(&server.fields.server_id));
}

#[tokio::test]
async fn build_a_pinned_issuer_anchors_the_metadata_fetch() {
    let fixture = Fixture::with_metadata();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "auth_type": "oauth2", "oauth2_flow": "authorization_code", "issuer": "https://as.test"}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert!(server.fields.issuer_is_anchored);
    assert_eq!(server.fields.issuer.as_deref(), Some("https://as.test"));
    assert_eq!(
        server.fields.authorization_url.as_deref(),
        Some("https://as.test/authorize")
    );
    assert_eq!(
        fixture.fetcher.calls().first().map(String::as_str),
        Some("https://as.test/.well-known/oauth-authorization-server")
    );
}

#[tokio::test]
async fn build_client_credentials_keeps_the_configured_endpoints() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "auth_type": "oauth2", "oauth2_flow": "client_credentials", "token_url": "https://idp.test/token", "authorization_url": "https://idp.test/authorize"}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(
        server.fields.oauth2_flow,
        Some(OAuth2Flow::ClientCredentials)
    );
    assert_eq!(
        server.fields.token_url.as_deref(),
        Some("https://idp.test/token")
    );
    assert_eq!(
        server.fields.configured_token_url.as_deref(),
        Some("https://idp.test/token")
    );
    assert_eq!(
        server.fields.authorization_url.as_deref(),
        Some("https://idp.test/authorize")
    );
}

#[tokio::test]
async fn build_blank_issuer_and_endpoints_are_treated_as_absent() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "issuer": "   ", "authorization_url": "", "token_url": "  "}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.issuer, None);
    assert_eq!(server.fields.authorization_url, None);
    assert_eq!(server.fields.token_url, None);
    assert_eq!(server.fields.configured_authorization_url, None);
    assert_eq!(server.fields.configured_token_url, None);
}

#[tokio::test]
async fn build_pinned_tools_are_parsed() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "pinned_tools": {"tool_a": {"description": "Describe"}}}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    let pinned = server.fields.pinned_tools.unwrap();
    let tool = &pinned["tool_a"];
    assert_eq!(tool.as_ref().description, "Describe");
}

#[tokio::test]
async fn build_a_short_prefix_is_assigned_when_enabled() {
    let mut fixture = Fixture::new();
    fixture.prefixes.short_prefix_enabled = true;
    fixture
        .load(&config(r#"{"test_server": {"url": "https://mcp.test"}}"#))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    let expected = compute_short_server_prefix(&server.fields.server_id, 0).unwrap();
    assert_eq!(server.fields.short_prefix, Some(expected));
}

#[tokio::test]
async fn build_a_used_prefix_forces_a_rehash() {
    let mut fixture = Fixture::new();
    fixture.prefixes.short_prefix_enabled = true;
    let pinned_id = "pinned-prefix-id";
    let taken = compute_short_server_prefix(pinned_id, 0).unwrap();
    fixture
        .add_db_server(json_server(&format!(
            r#"{{"server_id": "db-other", "name": "db_other", "transport": "http", "short_prefix": "{taken}"}}"#
        )))
        .await;
    fixture
        .load(&config(&format!(
            r#"{{"test_server": {{"url": "https://mcp.test", "server_id": "{pinned_id}"}}}}"#
        )))
        .await
        .unwrap();
    let server = fixture.server(pinned_id).await.unwrap();
    let expected = compute_short_server_prefix(pinned_id, 1).unwrap();
    assert_eq!(server.fields.short_prefix, Some(expected));
}

#[tokio::test]
async fn build_an_empty_alias_uses_the_server_name() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "alias": ""}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.name, "test_server");
}

#[tokio::test]
async fn build_warns_when_a_stdio_server_is_blocked() {
    let mut fixture = Fixture::new();
    fixture.stdio = FakeStdioPolicy(true);
    let (captured, _guard) = capture();
    fixture
        .load(&config(
            r#"{"test_server": {"transport": "stdio", "command": "npx"}}"#,
        ))
        .await
        .unwrap();
    assert!(captured.text().contains(&format!(
        "MCP server 'test_server' will not start: {MCP_STDIO_DISABLED_MESSAGE}"
    )));
}

#[tokio::test]
async fn build_does_not_warn_for_a_blocked_stdio_policy_on_http() {
    let mut fixture = Fixture::new();
    fixture.stdio = FakeStdioPolicy(true);
    let (captured, _guard) = capture();
    fixture
        .load(&config(r#"{"test_server": {"url": "https://mcp.test"}}"#))
        .await
        .unwrap();
    assert!(!captured.text().contains("will not start"));
}
