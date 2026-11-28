#[tokio::test]
async fn swap_replaces_db_servers_wholesale() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-old","name":"old","transport":"http"}"#,
        ))
        .await;
    fixture.rows.set(vec![row(PLAIN_ROW)]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    assert_eq!(fixture.db_ids().await, strings(&["srv-1"]));
    assert!(fixture.db_server("srv-old").await.is_none());
    assert_eq!(fixture.caches.invalidated(), strings(&["srv-1", "srv-old"]));
}

#[tokio::test]
async fn swap_invalidates_only_changed_ids() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-a","name":"old-a","transport":"http","url":"https://mcp.test","updated_at":"2024-01-01T00:00:00Z"}"#,
        ))
        .await;
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-b","name":"old-b","transport":"http","url":"https://mcp.test","updated_at":"2024-01-01T00:00:00Z"}"#,
        ))
        .await;
    fixture.rows.set(vec![
        row(
            r#"{"server_id":"srv-a","transport":"http","url":"https://mcp.test","alias":"fresh-a","updated_at":"2024-01-01T00:00:00Z"}"#,
        ),
        row(
            r#"{"server_id":"srv-b","transport":"http","url":"https://mcp.test","alias":"fresh-b","updated_at":"2024-01-02T00:00:00Z"}"#,
        ),
    ]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let held_a = fixture.db_server("srv-a").await.unwrap();
    let held_b = fixture.db_server("srv-b").await.unwrap();
    assert_eq!(held_a.fields.name, "old-a");
    assert_eq!(held_b.fields.name, "fresh-b");
    assert_eq!(fixture.caches.invalidated(), strings(&["srv-b"]));
}

#[tokio::test]
async fn swap_drops_discovery_state_for_missing_ids() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-gone","name":"gone","transport":"http","url":"https://mcp.test","auth_type":"oauth2"}"#,
        ))
        .await;
    fixture.discovery.set_deferred("srv-gone", true);
    fixture.rows.set(Vec::new());
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    assert!(fixture.db_ids().await.is_empty());
    assert!(!fixture.discovery.slot_exists("srv-gone"));
    assert_eq!(fixture.caches.invalidated(), strings(&["srv-gone"]));
}

#[tokio::test]
async fn swap_reconciles_discovery_slots_for_new_records() {
    let fixture = test_fixture();
    fixture.rows.set(vec![
        row(OAUTH_ROW),
        row(r#"{"server_id":"srv-2","transport":"http","url":"https://mcp.test"}"#),
    ]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    assert!(fixture.discovery.slot_exists("srv-1"));
    assert!(!fixture.discovery.slot_exists("srv-2"));
}

#[tokio::test]
async fn swap_primes_discovery_for_staged_servers() {
    let fixture = test_fixture();
    fixture.rows.set(vec![row(OAUTH_ROW)]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let calls = fixture.fetcher.calls();
    assert_eq!(calls.first().map(String::as_str), Some("https://mcp.test"));
}

#[tokio::test]
async fn swap_registers_openapi_tools_without_rebuilding_sink_mapping() {
    let fixture = test_fixture();
    let spec = corpus("petstore.json");
    let source = format!(
        r#"{{"server_id":"srv-1","transport":"http","url":"https://api.test","spec_path":"{spec}","server_name":"test_server"}}"#
    );
    fixture.rows.set(vec![row(&source)]);
    fixture
        .lister
        .plan("srv-1", ToolPlan::Tools(strings(&["test_server-listpets"])));
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    assert!(fixture.tools.tools().contains("test_server-listpets"));
    assert_eq!(fixture.tools.rebuilds(), 0);
    assert_eq!(
        fixture
            .tools
            .owners()
            .get("test_server-listpets")
            .map(String::as_str),
        Some("test_server")
    );
    fixture.settle().await;
    assert_eq!(fixture.lister.calls(), strings(&["srv-1"]));
    let owners = fixture.owners.read().unwrap().mapping.clone();
    assert_eq!(
        owners.get("listpets").map(String::as_str),
        Some("test_server")
    );
    assert_eq!(
        owners.get("test_server-listpets").map(String::as_str),
        Some("test_server")
    );
}

#[tokio::test]
async fn swap_skips_registration_for_empty_spec_path() {
    let fixture = test_fixture();
    fixture.rows.set(vec![row(
        r#"{"server_id":"srv-1","transport":"http","url":"https://api.test","spec_path":""}"#,
    )]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    assert!(fixture.tools.tools().is_empty());
    assert_eq!(fixture.tools.rebuilds(), 0);
    fixture.settle().await;
    assert!(fixture.lister.calls().is_empty());
}

#[tokio::test]
async fn swap_skips_initialization_without_openapi_tools() {
    let fixture = test_fixture();
    fixture.rows.set(vec![row(PLAIN_ROW)]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    fixture.settle().await;
    assert!(fixture.lister.calls().is_empty());
    assert!(fixture.tools.tools().is_empty());
}

#[tokio::test]
async fn swap_hydrates_config_servers() {
    let mut fixture = test_fixture();
    fixture.hydrator = FakeHydrator::returning(true);
    fixture
        .put_config_server(server(
            r#"{"server_id":"srv-cfg","name":"cfg","transport":"http"}"#,
        ))
        .await;
    fixture.rows.set(Vec::new());
    let (captured, _guard) = capture();
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let held = fixture.config_server("srv-cfg").await.unwrap();
    assert_eq!(held.fields.instructions.as_deref(), Some("hydrated"));
    assert_eq!(fixture.hydrator.calls(), strings(&["srv-cfg"]));
    assert!(
        captured
            .text()
            .contains("hydrated persisted DCR client onto config MCP server server_id=srv-cfg")
    );
}

#[tokio::test]
async fn swap_skips_hydration_when_dcr_returns_false() {
    let fixture = test_fixture();
    fixture
        .put_config_server(server(
            r#"{"server_id":"srv-cfg","name":"cfg","transport":"http"}"#,
        ))
        .await;
    fixture.rows.set(Vec::new());
    let (captured, _guard) = capture();
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let held = fixture.config_server("srv-cfg").await.unwrap();
    assert_eq!(held.fields.instructions, None);
    assert_eq!(fixture.hydrator.calls(), strings(&["srv-cfg"]));
    assert!(!captured.text().contains("hydrated persisted DCR client"));
}

#[tokio::test]
async fn swap_skips_write_back_when_config_server_disappears() {
    let mut fixture = test_fixture();
    let registry = Arc::clone(&fixture.registry);
    fixture.hydrator = FakeHydrator::returning(true).evicting(&registry);
    fixture
        .put_config_server(server(
            r#"{"server_id":"srv-cfg","name":"cfg","transport":"http"}"#,
        ))
        .await;
    fixture.rows.set(Vec::new());
    let (captured, _guard) = capture();
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    assert!(fixture.config_server("srv-cfg").await.is_none());
    assert_eq!(fixture.hydrator.calls(), strings(&["srv-cfg"]));
    assert!(!captured.text().contains("hydrated persisted DCR client"));
}
