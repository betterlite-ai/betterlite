#[test]
fn collision_config_ids_capturing_db_identifiers_are_reported() {
    let db = json_server(
        r#"{"server_id": "db-id", "name": "db-name", "transport": "http", "server_name": "db-server", "alias": "db-alias"}"#,
    );
    let ids: BTreeSet<String> = [
        String::from("db-id"),
        String::from("db-name"),
        String::from("db-server"),
        String::from("other"),
    ]
    .into_iter()
    .collect();
    let captured = config_ids_capturing_db_identifiers(&ids, &[db]);
    let expected: BTreeSet<String> = [String::from("db-name"), String::from("db-server")]
        .into_iter()
        .collect();
    assert_eq!(captured, expected);
}

#[tokio::test]
async fn collision_two_entries_pinning_one_id_is_rejected() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_a": {"url": "https://mcp.test", "server_id": "shared"}, "test_b": {"url": "https://mcp.test", "server_id": "shared"}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_b': server_id 'shared' is already used by MCP server 'test_a'. Each mcp_servers entry needs its own id."
    );
}

#[tokio::test]
async fn collision_a_derived_id_equal_to_an_earlier_pinned_id_is_rejected() {
    let fixture = Fixture::new();
    let derived = generate_stable_server_id("test_server", "https://mcp.test", None, None, None);
    let source = format!(
        r#"{{"test_pinned": {{"url": "https://mcp.test", "server_id": "{derived}"}}, "test_server": {{"url": "https://mcp.test"}}}}"#
    );
    let error = fixture.load(&config(&source)).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "Invalid config for MCP server 'test_server': server_id '{derived}' is already used by MCP server 'test_pinned'. Each mcp_servers entry needs its own id."
        )
    );
}

#[tokio::test]
async fn collision_a_pinned_id_owned_by_the_database_is_rejected() {
    let fixture = Fixture::new();
    fixture
        .add_db_server(json_server(
            r#"{"server_id": "db-id", "name": "db-name", "transport": "http"}"#,
        ))
        .await;
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "server_id": "db-id"}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_server': server_id 'db-id' belongs to a database-backed MCP server. The database entry takes precedence over config.yaml, so this server would never be reachable."
    );
}

#[tokio::test]
async fn collision_a_pinned_id_equal_to_another_entry_name_is_rejected() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_a": {"url": "https://mcp.test", "server_id": "test_b"}, "test_b": {"url": "https://mcp.test"}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_a': server_id 'test_b' is the server_name or alias of MCP server 'test_b'. Permission entries naming 'test_b' would resolve to 'test_a' alone and no longer reach 'test_b'."
    );
}

#[tokio::test]
async fn collision_a_pinned_id_equal_to_another_entry_mapped_alias_is_rejected() {
    let fixture = Fixture::new();
    let mapped = aliases(r#"{"other-alias": "test_b"}"#);
    let error = fixture
        .load_with(
            &config(
                r#"{"test_a": {"url": "https://mcp.test", "server_id": "other-alias"}, "test_b": {"url": "https://mcp.test"}}"#,
            ),
            Some(&mapped),
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_a': server_id 'other-alias' is the server_name or alias of MCP server 'test_b'. Permission entries naming 'other-alias' would resolve to 'test_a' alone and no longer reach 'test_b'."
    );
}

#[tokio::test]
async fn collision_a_pinned_id_equal_to_an_identifier_shared_by_two_entries_is_rejected() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_a": {"url": "https://mcp.test", "alias": "shared", "server_id": "shared"}, "test_b": {"url": "https://mcp.test", "alias": "shared"}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_a': server_id 'shared' is the server_name or alias of MCP server 'test_b'. Permission entries naming 'shared' would resolve to 'test_a' alone and no longer reach 'test_b'."
    );
}

#[tokio::test]
async fn collision_pinning_an_identifier_only_this_entry_owns_is_allowed() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_a": {"url": "https://mcp.test", "alias": "test_a", "server_id": "test_a"}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server("test_a").await.unwrap();
    assert_eq!(server.fields.server_name.as_deref(), Some("test_a"));
}

#[tokio::test]
async fn collision_derived_ids_never_collide() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_a": {"url": "https://mcp.test"}, "test_b": {"url": "https://mcp.test"}}"#,
        ))
        .await
        .unwrap();
    assert_eq!(fixture.servers().await.len(), 2);
}

#[tokio::test]
async fn validation_a_server_name_with_the_separator_is_rejected() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(r#"{"test_server-x": {"url": "https://mcp.test"}}"#))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Server name cannot contain '-'. Use an alternative character instead Found: test_server-x"
    );
}

#[tokio::test]
async fn validation_oauth2_without_a_flow_is_rejected() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "auth_type": "oauth2"}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_server': auth_type oauth2 requires an explicit oauth2_flow (got None). Set oauth2_flow: client_credentials for machine-to-machine servers (the proxy mints a shared token at token_url using client_id/client_secret, no user interaction) or oauth2_flow: authorization_code for interactive servers (per-user tokens via browser sign-in, including delegate_auth_to_upstream)."
    );
}

#[tokio::test]
async fn validation_oauth2_with_an_unknown_flow_is_rejected() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "auth_type": "oauth2", "oauth2_flow": "implicit"}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_server': auth_type oauth2 requires an explicit oauth2_flow (got 'implicit'). Set oauth2_flow: client_credentials for machine-to-machine servers (the proxy mints a shared token at token_url using client_id/client_secret, no user interaction) or oauth2_flow: authorization_code for interactive servers (per-user tokens via browser sign-in, including delegate_auth_to_upstream)."
    );
}

#[tokio::test]
async fn validation_per_server_oauth_discovery_null_is_rejected() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "per_server_oauth_discovery": null}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_server': per_server_oauth_discovery must be a boolean (got None)."
    );
}

#[tokio::test]
async fn validation_per_server_oauth_discovery_must_be_a_boolean() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "per_server_oauth_discovery": "yes"}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_server': per_server_oauth_discovery must be a boolean (got 'yes')."
    );
}

#[tokio::test]
async fn validation_per_server_oauth_discovery_needs_authorization_code() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "auth_type": "oauth2", "oauth2_flow": "client_credentials", "per_server_oauth_discovery": true}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_server': per_server_oauth_discovery is only supported for auth_type oauth2 with oauth2_flow authorization_code and without delegate_auth_to_upstream."
    );
}

#[tokio::test]
async fn validation_per_server_oauth_discovery_rejects_delegate_auth() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "auth_type": "oauth2", "oauth2_flow": "authorization_code", "delegate_auth_to_upstream": true, "per_server_oauth_discovery": true}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_server': per_server_oauth_discovery is only supported for auth_type oauth2 with oauth2_flow authorization_code and without delegate_auth_to_upstream."
    );
}

#[tokio::test]
async fn validation_dcr_bridge_must_be_a_boolean() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "dcr_bridge": "yes"}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_server': dcr_bridge must be a boolean (got 'yes')."
    );
}

#[tokio::test]
async fn validation_dcr_bridge_needs_passthrough_or_delegate() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "auth_type": "bearer_token", "dcr_bridge": true}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_server': dcr_bridge is only supported for auth_type true_passthrough or oauth_delegate (got 'bearer_token'). The DCR bridge serves gateway-hosted OAuth discovery for the client-forwarded token modes; interactive oauth2 servers already run the gateway authorization-code flow."
    );
}

#[tokio::test]
async fn validation_dcr_bridge_false_is_allowed_for_any_auth_type() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "auth_type": "bearer_token", "dcr_bridge": false}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.dcr_bridge, Some(false));
}

#[tokio::test]
async fn validation_dcr_bridge_true_is_recorded_for_true_passthrough() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "auth_type": "true_passthrough", "dcr_bridge": true}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.dcr_bridge, Some(true));
}

#[tokio::test]
async fn validation_token_validation_must_be_an_object() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "token_validation": "audience"}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_server': token_validation must be an object."
    );
}

#[tokio::test]
async fn validation_mcp_info_protocol_version_is_checked() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "mcp_info": {"protocol_version": "1999"}}}"#,
        ))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("Invalid config for MCP server 'test_server': ")
    );
}

#[tokio::test]
async fn validation_errors_use_the_derived_id_when_the_name_is_empty() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"": {"url": "https://mcp.test", "auth_type": "oauth2"}}"#,
        ))
        .await
        .unwrap_err();
    let server_id =
        generate_stable_server_id("", "https://mcp.test", None, Some(MCPAuth::Oauth2), None);
    assert!(error.to_string().starts_with(&format!(
        "Invalid config for MCP server '{server_id}': auth_type oauth2"
    )));
}
