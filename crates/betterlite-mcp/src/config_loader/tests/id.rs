#[test]
fn id_matches_the_python_hash_vector() {
    assert_eq!(
        generate_stable_server_id("test-server", "https://mcp.test", None, None, None),
        "d831b3f55d0103500e200ac411b57975"
    );
}

#[test]
fn id_matches_the_python_hash_vector_with_transport_and_auth_type() {
    assert_eq!(
        generate_stable_server_id(
            "test-server",
            "https://mcp.test",
            Some(MCPTransport::Http),
            Some(MCPAuth::Oauth2),
            None,
        ),
        "86428a765359df93228cd0d81cf6073d"
    );
}

#[test]
fn id_matches_the_python_hash_vector_for_an_empty_name() {
    assert_eq!(
        generate_stable_server_id("", "https://mcp.test", None, None, None),
        "e58df9d46c4d493e8afd90bd5584e534"
    );
}

#[test]
fn id_every_identifying_field_changes_the_hash() {
    let base = generate_stable_server_id("test-server", "https://mcp.test", None, None, None);
    let transport = generate_stable_server_id(
        "test-server",
        "https://mcp.test",
        Some(MCPTransport::Http),
        None,
        None,
    );
    let auth = generate_stable_server_id(
        "test-server",
        "https://mcp.test",
        None,
        Some(MCPAuth::Oauth2),
        None,
    );
    let alias = generate_stable_server_id(
        "test-server",
        "https://mcp.test",
        None,
        None,
        Some("test-alias"),
    );
    assert_ne!(base, transport);
    assert_ne!(base, auth);
    assert_ne!(base, alias);
}

#[test]
fn id_pinned_config_ids_are_trimmed_and_absent_is_none() {
    assert_eq!(pinned_config_server_id(None, "test-server").unwrap(), None);
    let raw = Json::Text(String::from("  pinned-id  "));
    assert_eq!(
        pinned_config_server_id(Some(&raw), "test-server").unwrap(),
        Some(String::from("pinned-id"))
    );
}

#[tokio::test]
async fn id_a_pinned_id_becomes_the_registry_key() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "server_id": "  pinned-id  "}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server("pinned-id").await.unwrap();
    assert_eq!(server.fields.server_id, "pinned-id");
    assert_eq!(server.fields.server_name.as_deref(), Some("test_server"));
}

#[tokio::test]
async fn id_a_blank_pinned_id_is_rejected() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "server_id": "   "}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_server': server_id must be a non-empty string."
    );
}

#[tokio::test]
async fn id_a_non_string_pinned_id_is_rejected() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "server_id": 42}}"#,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid config for MCP server 'test_server': server_id must be a non-empty string."
    );
}

#[tokio::test]
async fn id_uses_the_mapped_alias_in_the_hash_and_the_name() {
    let fixture = Fixture::new();
    let mapped = aliases(r#"{"test_alias": "test_server"}"#);
    fixture
        .load_with(
            &config(r#"{"test_server": {"url": "https://mcp.test"}}"#),
            Some(&mapped),
        )
        .await
        .unwrap();
    let expected = generate_stable_server_id(
        "test_server",
        "https://mcp.test",
        None,
        None,
        Some("test_alias"),
    );
    let server = fixture.server(&expected).await.unwrap();
    assert_eq!(server.fields.name, "test_alias");
    assert_eq!(server.fields.alias.as_deref(), Some("test_alias"));
}
