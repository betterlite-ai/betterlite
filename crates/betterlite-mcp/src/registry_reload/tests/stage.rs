#[tokio::test]
async fn stage_reuses_unchanged_record_with_slot() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"previous-name","transport":"http","url":"https://mcp.test","auth_type":"oauth2","updated_at":"2024-01-01T00:00:00Z"}"#,
        ))
        .await;
    fixture.discovery.set_deferred("srv-1", true);
    fixture.rows.set(vec![row(
        r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","auth_type":"oauth2","alias":"fresh","updated_at":"2024-01-01T00:00:00Z"}"#,
    )]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let held = fixture.db_server("srv-1").await.unwrap();
    assert_eq!(held.fields.name, "previous-name");
    assert_eq!(held.fields.alias, None);
}

#[tokio::test]
async fn stage_reuses_resolved_record_without_slot() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"previous-name","transport":"http","url":"https://mcp.test","updated_at":"2024-01-01T00:00:00Z"}"#,
        ))
        .await;
    fixture.rows.set(vec![row(
        r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","alias":"fresh","updated_at":"2024-01-01T00:00:00"}"#,
    )]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let held = fixture.db_server("srv-1").await.unwrap();
    assert_eq!(held.fields.name, "previous-name");
}

#[tokio::test]
async fn stage_rebuilds_unresolved_record_without_slot() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"previous-name","transport":"http","url":"https://mcp.test","auth_type":"oauth2","updated_at":"2024-01-01T00:00:00Z"}"#,
        ))
        .await;
    fixture.rows.set(vec![row(
        r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","auth_type":"oauth2","alias":"fresh","updated_at":"2024-01-01T00:00:00Z"}"#,
    )]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let held = fixture.db_server("srv-1").await.unwrap();
    assert_eq!(held.fields.name, "fresh");
}

#[tokio::test]
async fn stage_rebuilds_when_updated_at_differs() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"previous-name","transport":"http","url":"https://mcp.test","updated_at":"2024-01-01T00:00:00Z"}"#,
        ))
        .await;
    fixture.discovery.set_deferred("srv-1", true);
    fixture.rows.set(vec![row(
        r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","alias":"fresh","updated_at":"2024-01-02T00:00:00Z"}"#,
    )]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let held = fixture.db_server("srv-1").await.unwrap();
    assert_eq!(held.fields.name, "fresh");
}

#[tokio::test]
async fn stage_rebuilds_when_updated_at_is_missing() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"previous-name","transport":"http","url":"https://mcp.test","updated_at":"2024-01-01T00:00:00Z"}"#,
        ))
        .await;
    fixture.rows.set(vec![row(
        r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","alias":"fresh"}"#,
    )]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let held = fixture.db_server("srv-1").await.unwrap();
    assert_eq!(held.fields.name, "fresh");
}

#[tokio::test]
async fn stage_keeps_previous_short_prefix_on_rebuild() {
    let fixture = test_fixture();
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"previous-name","transport":"http","url":"https://mcp.test","auth_type":"oauth2","short_prefix":"pinned","authorization_url":"https://as.test/authorize","token_url":"https://as.test/token","updated_at":"2024-01-01T00:00:00Z"}"#,
        ))
        .await;
    fixture.rows.set(vec![row(
        r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","auth_type":"oauth2","alias":"fresh","updated_at":"2024-01-02T00:00:00Z"}"#,
    )]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let held = fixture.db_server("srv-1").await.unwrap();
    assert_eq!(held.fields.name, "fresh");
    assert_eq!(held.fields.short_prefix.as_deref(), Some("pinned"));
    assert_eq!(
        held.fields.token_url.as_deref(),
        Some("https://as.test/token")
    );
}

#[tokio::test]
async fn stage_skips_bad_row_and_keeps_the_rest() {
    let fixture = test_fixture();
    let (captured, _guard) = capture();
    fixture.rows.set(vec![
        row(PLAIN_ROW),
        row(
            r#"{"server_id":"srv-bad","transport":"http","alias":"bad-row","credentials":{"upstream_token_header":"bad header"}}"#,
        ),
    ]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    assert_eq!(fixture.db_ids().await, strings(&["srv-1"]));
    assert!(
        captured
            .text()
            .contains("Skipping MCP server srv-bad (bad-row) during DB reload:")
    );
}

#[tokio::test]
async fn stage_skips_spec_registration_failure() {
    let fixture = test_fixture();
    let (captured, _guard) = capture();
    let missing = format!(
        "{}/target/b6765-missing-openapi-spec.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let source = format!(
        r#"{{"server_id":"srv-1","transport":"http","url":"https://api.test","spec_path":"{missing}"}}"#
    );
    fixture.rows.set(vec![row(&source)]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    assert!(fixture.db_ids().await.is_empty());
    assert!(
        captured
            .text()
            .contains("Skipping MCP server srv-1 (None) during DB reload:")
    );
}

#[tokio::test]
async fn stage_assigns_prefixes_across_full_staged_set() {
    let mut fixture = test_fixture();
    fixture.prefixes = PrefixPolicy {
        separator: String::from("-"),
        short_prefix_enabled: true,
    };
    let (first, second) = colliding_ids();
    fixture.rows.set(vec![
        row(&format!(
            r#"{{"server_id":"{first}","transport":"http","url":"https://mcp.test"}}"#
        )),
        row(&format!(
            r#"{{"server_id":"{second}","transport":"http","url":"https://mcp.test"}}"#
        )),
    ]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let natural = compute_short_server_prefix(&first, 0).unwrap();
    let rehashed = compute_short_server_prefix(&second, 1).unwrap();
    let held_first = fixture.db_server(&first).await.unwrap();
    let held_second = fixture.db_server(&second).await.unwrap();
    assert_eq!(
        held_first.fields.short_prefix.as_deref(),
        Some(natural.as_str())
    );
    assert_eq!(
        held_second.fields.short_prefix.as_deref(),
        Some(rehashed.as_str())
    );
}

#[tokio::test]
async fn stage_decrypts_encrypted_env_vars_on_reload() {
    let fixture = test_fixture();
    let sealed = encrypt(&fixture.cipher, "s3cret");
    let value = serde_json::to_string(&sealed).unwrap();
    let source = format!(
        r#"{{"server_id":"srv-1","transport":"http","url":"https://mcp.test","env_vars":[{{"name":"TOKEN","value":{value}}}]}}"#
    );
    fixture.rows.set(vec![row(&source)]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let held = fixture.db_server("srv-1").await.unwrap();
    assert_eq!(
        env_value(&held, "TOKEN"),
        Some(JsonValue::String(String::from("s3cret")))
    );
}

#[tokio::test]
async fn stage_logs_found_count_and_clears_instructions() {
    let fixture = test_fixture();
    let (captured, _guard) = capture();
    fixture.rows.set(vec![
        row(PLAIN_ROW),
        row(
            r#"{"server_id":"srv-2","transport":"http","url":"https://mcp.test","server_name":"two"}"#,
        ),
    ]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let text = captured.text();
    assert!(text.contains("Loading MCP servers from database into registry..."));
    assert!(text.contains("Found 2 MCP servers in database"));
    assert!(text.contains("Building server from DB: srv-1 (None)"));
    assert!(text.contains("Building server from DB: srv-2 (two)"));
    assert!(text.contains("MCP registry refreshed (2 servers in registry)"));
    assert_eq!(fixture.instructions.clears(), 1);
    assert_eq!(
        fixture.gate_slots.entries(),
        vec![
            (String::from("srv-1"), false),
            (String::from("srv-2"), false),
        ]
    );
}

#[tokio::test]
async fn stage_warns_on_invalid_server_name_field() {
    let fixture = test_fixture();
    let (captured, _guard) = capture();
    fixture.rows.set(vec![row(
        r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","alias":"bad alias"}"#,
    )]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    assert!(
        captured
            .text()
            .contains("MCP server 'srv-1' has invalid alias 'bad alias':")
    );
}

#[tokio::test]
async fn stage_warns_when_stdio_becomes_blocked() {
    let mut fixture = test_fixture();
    fixture.stdio.blocked = true;
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-1","name":"probe","transport":"http","url":"https://mcp.test"}"#,
        ))
        .await;
    let (captured, _guard) = capture();
    fixture.rows.set(vec![row(
        r#"{"server_id":"srv-1","transport":"stdio","command":"npx","alias":"probe"}"#,
    )]);
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    assert!(
        captured
            .text()
            .contains("MCP server 'probe' will not start:")
    );
}

#[tokio::test]
async fn stage_propagates_row_read_error() {
    let fixture = test_fixture();
    fixture.rows.fail_with("srv-err");
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    let error = mutator.reload().await.unwrap_err();
    assert!(matches!(error, MutationError::Rows(_)));
    assert!(fixture.db_ids().await.is_empty());
    assert_eq!(fixture.instructions.clears(), 1);
}
