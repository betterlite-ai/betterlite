#[tokio::test]
async fn mutation_add_ignores_ineligible_status() {
    let fixture = test_fixture();
    let (captured, _guard) = capture();
    let table = row(
        r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","approval_status":"pending"}"#,
    );
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.add(&table).await.unwrap();
    assert!(fixture.db_ids().await.is_empty());
    assert!(fixture.gate_slots.entries().is_empty());
    assert!(fixture.caches.invalidated().is_empty());
    assert!(!captured.text().contains("Added MCP Server"));
}

#[tokio::test]
async fn mutation_add_leaves_registered_server_untouched() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"old-name","transport":"http"}"#,
        ))
        .await;
    let table =
        row(r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","alias":"fresh"}"#);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.add(&table).await.unwrap();
    let held = fixture.db_server("srv-1").await.unwrap();
    assert_eq!(held.fields.name, "old-name");
    assert!(fixture.caches.invalidated().is_empty());
}

#[tokio::test]
async fn mutation_add_inserts_and_assigns_prefix() {
    let mut fixture = test_fixture();
    fixture.prefixes = PrefixPolicy {
        separator: String::from("-"),
        short_prefix_enabled: true,
    };
    let (captured, _guard) = capture();
    let table =
        row(r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","alias":"added"}"#);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.add(&table).await.unwrap();
    mutator.add(&table).await.unwrap();
    let held = fixture.db_server("srv-1").await.unwrap();
    let natural = compute_short_server_prefix("srv-1", 0).unwrap();
    assert_eq!(held.fields.name, "added");
    assert_eq!(held.fields.short_prefix.as_deref(), Some(natural.as_str()));
    assert_eq!(fixture.caches.invalidated(), strings(&["srv-1"]));
    assert!(captured.text().contains("Added MCP Server: added"));
}

#[tokio::test]
async fn mutation_add_builds_with_plain_env_vars() {
    let fixture = test_fixture();
    let sealed = encrypt(&fixture.cipher, "s3cret");
    let source = serde_json::json!({
        "server_id": "srv-1",
        "transport": "http",
        "url": "https://mcp.test",
        "env_vars": [{"name": "TOKEN", "value": sealed.clone()}],
    })
    .to_string();
    let table = row(&source);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.add(&table).await.unwrap();
    let held = fixture.db_server("srv-1").await.unwrap();
    assert_eq!(env_value(&held, "TOKEN"), Some(JsonValue::String(sealed)));
}

#[tokio::test]
async fn mutation_add_returns_build_failure_after_the_debug_log() {
    let fixture = test_fixture();
    let (captured, _guard) = capture();
    let table = row(
        r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","credentials":{"upstream_token_header":"bad header"}}"#,
    );
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    let error = mutator.add(&table).await.unwrap_err();
    assert!(matches!(error, MutationError::Build(_)));
    assert!(fixture.db_ids().await.is_empty());
    assert!(captured.text().contains("Failed to add MCP server:"));
}

#[tokio::test]
async fn mutation_update_returns_build_failure_after_the_debug_log() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"before","transport":"http"}"#,
        ))
        .await;
    let (captured, _guard) = capture();
    let table = row(
        r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","credentials":{"upstream_token_header":"bad header"}}"#,
    );
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    let error = mutator.update(&table).await.unwrap_err();
    assert!(matches!(error, MutationError::Build(_)));
    let held = fixture.db_server("srv-1").await.unwrap();
    assert_eq!(held.fields.name, "before");
    assert!(captured.text().contains("Failed to udpate MCP server:"));
}

#[tokio::test]
async fn mutation_update_evicts_when_ineligible() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"old-name","transport":"http"}"#,
        ))
        .await;
    fixture.discovery.set_deferred("srv-1", true);
    let (captured, _guard) = capture();
    let table = row(r#"{"server_id":"srv-1","transport":"http","approval_status":"rejected"}"#);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.update(&table).await.unwrap();
    assert!(fixture.db_ids().await.is_empty());
    assert_eq!(fixture.caches.invalidated(), strings(&["srv-1"]));
    assert!(!fixture.discovery.slot_exists("srv-1"));
    assert!(!captured.text().contains("Removed MCP Server"));
}

#[tokio::test]
async fn mutation_update_evicts_by_server_name() {
    let fixture = test_fixture();
    fixture
        .put_db_entry(
            "legacy-name",
            server(r#"{"server_id":"srv-9","name":"legacy-name","transport":"http"}"#),
        )
        .await;
    fixture.discovery.set_deferred("srv-9", true);
    let table = row(
        r#"{"server_id":"srv-new","transport":"http","server_name":"legacy-name","approval_status":"pending"}"#,
    );
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.update(&table).await.unwrap();
    assert!(fixture.db_ids().await.is_empty());
    assert_eq!(fixture.caches.invalidated(), strings(&["srv-9"]));
    assert!(!fixture.discovery.slot_exists("srv-9"));
}

#[tokio::test]
async fn mutation_update_ignores_unregistered_id() {
    let fixture = test_fixture();
    let (captured, _guard) = capture();
    let table =
        row(r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","alias":"fresh"}"#);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.update(&table).await.unwrap();
    assert!(fixture.db_ids().await.is_empty());
    assert!(fixture.caches.invalidated().is_empty());
    assert!(!captured.text().contains("Updated MCP Server"));
}

#[tokio::test]
async fn mutation_update_keeps_prefix_and_carries_endpoints() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"before","transport":"http","url":"https://mcp.test","auth_type":"oauth2","short_prefix":"pin","authorization_url":"https://as.test/authorize","token_url":"https://as.test/token"}"#,
        ))
        .await;
    let (captured, _guard) = capture();
    let table = row(
        r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","auth_type":"oauth2","alias":"after"}"#,
    );
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.update(&table).await.unwrap();
    let held = fixture.db_server("srv-1").await.unwrap();
    assert_eq!(held.fields.name, "after");
    assert_eq!(held.fields.short_prefix.as_deref(), Some("pin"));
    assert_eq!(
        held.fields.token_url.as_deref(),
        Some("https://as.test/token")
    );
    assert_eq!(
        held.fields.authorization_url.as_deref(),
        Some("https://as.test/authorize")
    );
    assert_eq!(fixture.caches.invalidated(), strings(&["srv-1"]));
    assert!(captured.text().contains("Updated MCP Server: after"));
}

#[tokio::test]
async fn mutation_remove_pops_by_id_and_warns_missing() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"subject","transport":"http"}"#,
        ))
        .await;
    fixture.discovery.set_deferred("srv-1", true);
    let (captured, _guard) = capture();
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator
        .remove(&row(r#"{"server_id":"srv-1","transport":"http"}"#))
        .await;
    assert!(fixture.db_ids().await.is_empty());
    assert!(!fixture.discovery.slot_exists("srv-1"));
    assert_eq!(fixture.caches.invalidated(), strings(&["srv-1"]));
    assert!(captured.text().contains("Removed MCP Server: srv-1"));
    mutator
        .remove(&row(r#"{"server_id":"missing","transport":"http"}"#))
        .await;
    assert!(
        captured
            .text()
            .contains("Server ID missing not found in registry")
    );
}

#[tokio::test]
async fn mutation_remove_falls_back_to_server_name() {
    let fixture = test_fixture();
    fixture
        .put_db_entry(
            "legacy",
            server(r#"{"server_id":"srv-9","name":"legacy","transport":"http"}"#),
        )
        .await;
    let (captured, _guard) = capture();
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator
        .remove(&row(
            r#"{"server_id":"other","transport":"http","server_name":"legacy"}"#,
        ))
        .await;
    assert!(fixture.db_ids().await.is_empty());
    assert_eq!(fixture.caches.invalidated(), strings(&["srv-9"]));
    assert!(captured.text().contains("Removed MCP Server: other"));
    fixture
        .put_db_entry(
            "fresh",
            server(r#"{"server_id":"srv-10","name":"fresh","transport":"http"}"#),
        )
        .await;
    mutator
        .remove(&row(
            r#"{"server_id":"","transport":"http","server_name":"fresh"}"#,
        ))
        .await;
    assert!(fixture.db_ids().await.is_empty());
    assert!(captured.text().contains("Removed MCP Server: fresh"));
}

#[tokio::test]
async fn mutation_add_rehashes_around_a_registered_short_prefix() {
    let mut fixture = test_fixture();
    fixture.prefixes = PrefixPolicy {
        separator: String::from("-"),
        short_prefix_enabled: true,
    };
    let (first, second) = colliding_ids();
    let natural = compute_short_server_prefix(&first, 0).unwrap();
    let config_source = format!(
        r#"{{"server_id":"{first}","name":"cfg","transport":"http","short_prefix":"{natural}"}}"#
    );
    fixture.put_config_server(server(&config_source)).await;
    let source = format!(
        r#"{{"server_id":"{second}","transport":"http","url":"https://mcp.test","alias":"added"}}"#
    );
    let table = row(&source);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.add(&table).await.unwrap();
    let held = fixture.db_server(&second).await.unwrap();
    let rehashed = compute_short_server_prefix(&second, 1).unwrap();
    assert_eq!(held.fields.short_prefix.as_deref(), Some(rehashed.as_str()));
}

#[tokio::test]
async fn mutation_initialize_maps_original_and_listed_names() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"prefix-one","transport":"http","url":"https://mcp.test","alias":"prefix-one"}"#,
        ))
        .await;
    fixture.lister.plan(
        "srv-1",
        ToolPlan::Tools(strings(&["prefix-one-echo", "bare"])),
    );
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.initialize_tool_owners();
    fixture.settle().await;
    assert_eq!(fixture.lister.calls(), strings(&["srv-1"]));
    let owners = fixture.owners.read().unwrap().mapping.clone();
    assert_eq!(owners.get("echo").map(String::as_str), Some("prefix-one"));
    assert_eq!(
        owners.get("prefix-one-echo").map(String::as_str),
        Some("prefix-one")
    );
    assert_eq!(owners.get("bare").map(String::as_str), Some("prefix-one"));
}

#[tokio::test]
async fn mutation_initialize_skips_slot_and_user_token_servers() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-slot","name":"slot","transport":"http","url":"https://mcp.test"}"#,
        ))
        .await;
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-oauth","name":"oauth","transport":"http","auth_type":"oauth2"}"#,
        ))
        .await;
    fixture.discovery.set_deferred("srv-slot", true);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.initialize_tool_owners();
    fixture.settle().await;
    assert!(fixture.lister.calls().is_empty());
}

#[tokio::test]
async fn mutation_initialize_skips_upstream_auth_error() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"srv-1","transport":"http","url":"https://mcp.test"}"#,
        ))
        .await;
    fixture.lister.plan("srv-1", ToolPlan::UpstreamAuth);
    let (captured, _guard) = capture();
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.initialize_tool_owners();
    fixture.settle().await;
    assert!(
        captured
            .text()
            .contains("Skipping tool name mapping for server srv-1 due to upstream auth error:")
    );
    assert!(fixture.owners.read().unwrap().mapping.is_empty());
}

#[tokio::test]
async fn mutation_initialize_warns_on_tool_list_failure() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"srv-1","transport":"http","url":"https://mcp.test"}"#,
        ))
        .await;
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-2","name":"srv-2","transport":"http","url":"https://mcp.test"}"#,
        ))
        .await;
    fixture.lister.plan("srv-1", ToolPlan::Failed);
    fixture
        .lister
        .plan("srv-2", ToolPlan::Tools(strings(&["srv-2-echo"])));
    let (captured, _guard) = capture();
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.initialize_tool_owners();
    fixture.settle().await;
    assert!(captured.text().contains(
        "Failed to get tools from server srv-1 during tool name mapping initialization: boom"
    ));
    assert_eq!(fixture.lister.calls(), strings(&["srv-1", "srv-2"]));
    let owners = fixture.owners.read().unwrap().mapping.clone();
    assert_eq!(owners.get("echo").map(String::as_str), Some("srv-2"));
}
