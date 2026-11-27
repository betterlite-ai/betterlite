#[tokio::test]
async fn side_instructions_cache_is_cleared_once_per_load() {
    let fixture = Fixture::new();
    let minimal = config(r#"{"test_server": {"url": "https://mcp.test"}}"#);
    fixture.load(&minimal).await.unwrap();
    assert_eq!(fixture.instructions.clears(), 1);
    fixture.load(&minimal).await.unwrap();
    assert_eq!(fixture.instructions.clears(), 2);
}

#[tokio::test]
async fn side_caches_are_invalidated_before_each_insert() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_a": {"url": "https://mcp.test"}, "test_b": {"url": "https://mcp.test"}}"#,
        ))
        .await
        .unwrap();
    let expected: Vec<String> = fixture
        .servers()
        .await
        .iter()
        .map(|server| server.fields.server_id.clone())
        .collect();
    assert_eq!(fixture.caches.invalidated(), expected);
}

#[tokio::test]
async fn side_a_failed_entry_leaves_the_earlier_servers_loaded() {
    let fixture = Fixture::new();
    let error = fixture
        .load(&config(
            r#"{"test_first": {"url": "https://mcp.test"}, "test_second": {"url": "https://mcp.test", "auth_type": "oauth2"}}"#,
        ))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("test_second"));
    let servers = fixture.servers().await;
    assert_eq!(servers.len(), 1);
    assert_eq!(
        servers.first().unwrap().fields.server_name.as_deref(),
        Some("test_first")
    );
    assert_eq!(fixture.caches.invalidated().len(), 1);
}

#[tokio::test]
async fn side_the_redacted_dump_masks_every_secret() {
    let fixture = Fixture::new();
    let (captured, _guard) = capture();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "authentication_token": "top-secret-token", "client_secret": "client-secret", "client_private_key": "private-key", "aws_secret_access_key": "aws-secret"}}"#,
        ))
        .await
        .unwrap();
    let text = captured.text();
    assert!(text.contains("Loaded MCP Servers:"));
    assert!(text.contains(REDACTED_VALUE));
    assert!(!text.contains("top-secret-token"));
    assert!(!text.contains("client-secret"));
    assert!(!text.contains("private-key"));
    assert!(!text.contains("aws-secret"));
}

#[test]
fn side_redacted_dump_leaves_absent_secrets_as_none() {
    let server = json_server(
        r#"{"server_id": "test-id", "name": "test_server", "transport": "http", "authentication_token": "tok"}"#,
    );
    let mut servers = IndexMap::new();
    servers.insert(String::from("test-id"), server);
    let dump = redacted_registry_dump(&servers);
    let fields = dump.get("test-id").unwrap();
    assert_eq!(fields.get("authentication_token").unwrap(), REDACTED_VALUE);
    assert_eq!(fields.get("client_secret").unwrap(), "None");
    assert_eq!(fields.get("client_private_key").unwrap(), "None");
    assert_eq!(fields.get("name").unwrap(), "test_server");
}

#[tokio::test]
async fn side_dcr_hydration_writes_back_a_hydrated_server() {
    let mut fixture = Fixture::new();
    fixture.dcr = FakeDcrHydrator::returning(true);
    let (captured, _guard) = capture();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "instructions": "original"}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.instructions.as_deref(), Some("hydrated"));
    assert_eq!(
        fixture.dcr.hydrated(),
        vec![server.fields.server_id.clone()]
    );
    assert!(captured.text().contains(&format!(
        "hydrated persisted DCR client onto config MCP server server_id={}",
        server.fields.server_id
    )));
}

#[tokio::test]
async fn side_dcr_hydration_keeps_the_server_when_nothing_is_hydrated() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "instructions": "original"}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.instructions.as_deref(), Some("original"));
    assert_eq!(
        fixture.dcr.hydrated(),
        vec![server.fields.server_id.clone()]
    );
}

#[tokio::test]
async fn side_tool_name_mapping_is_rebuilt_once_after_the_load() {
    let fixture = Fixture::new();
    let minimal = config(r#"{"test_server": {"url": "https://mcp.test"}}"#);
    fixture.load(&minimal).await.unwrap();
    assert_eq!(fixture.tools.rebuilds(), 1);
    fixture.load(&minimal).await.unwrap();
    assert_eq!(fixture.tools.rebuilds(), 2);
}

#[tokio::test]
async fn side_a_deferred_discovery_slot_is_recorded_only_for_discovery_servers() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_oauth": {"url": "https://mcp.test", "auth_type": "oauth2", "oauth2_flow": "authorization_code"}, "test_key": {"url": "https://mcp.test", "auth_type": "api_key", "authentication_token": "tok"}}"#,
        ))
        .await
        .unwrap();
    let oauth = fixture.server_named("test_oauth").await.unwrap();
    assert!(fixture.discovery.slot_exists(&oauth.fields.server_id));
    let key = fixture.server_named("test_key").await.unwrap();
    assert!(!fixture.discovery.slot_exists(&key.fields.server_id));
}
