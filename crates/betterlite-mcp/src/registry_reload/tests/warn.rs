const SHADOW_WARNING: &str = "are also database-backed MCP servers";
const CAPTURE_WARNING: &str = "are the name or alias of a database-backed MCP server";

#[tokio::test]
async fn warn_shadowed_config_id_logs_once_and_again_after_change() {
    let fixture = test_fixture();
    fixture
        .put_config_server(server(
            r#"{"server_id":"srv-shared","name":"srv-shared","transport":"http"}"#,
        ))
        .await;
    fixture
        .put_db_server(server(
            r#"{"server_id":"srv-shared","name":"srv-shared","transport":"http"}"#,
        ))
        .await;
    fixture.rows.set(vec![row(
        r#"{"server_id":"srv-shared","transport":"http"}"#,
    )]);
    let (captured, _guard) = capture();
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    assert_eq!(occurrences(&captured.text(), SHADOW_WARNING), 1);
    assert_eq!(occurrences(&captured.text(), CAPTURE_WARNING), 0);
    mutator.reload().await.unwrap();
    assert_eq!(occurrences(&captured.text(), SHADOW_WARNING), 1);
    fixture
        .put_config_server(server(
            r#"{"server_id":"srv-other","name":"srv-other","transport":"http"}"#,
        ))
        .await;
    fixture.rows.set(vec![
        row(r#"{"server_id":"srv-shared","transport":"http"}"#),
        row(r#"{"server_id":"srv-other","transport":"http"}"#),
    ]);
    mutator.reload().await.unwrap();
    let text = captured.text();
    assert_eq!(occurrences(&text, SHADOW_WARNING), 2);
    assert!(text.contains("config.yaml MCP server_id(s) srv-other, srv-shared are also"));
}

#[tokio::test]
async fn warn_capturing_config_id_logs_when_db_name_matches() {
    let fixture = test_fixture();
    fixture
        .put_config_server(server(
            r#"{"server_id":"cfg-alias","name":"cfg-alias","transport":"http"}"#,
        ))
        .await;
    fixture.rows.set(vec![row(
        r#"{"server_id":"srv-db","transport":"http","url":"https://mcp.test","alias":"cfg-alias"}"#,
    )]);
    let (captured, _guard) = capture();
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let text = captured.text();
    assert_eq!(occurrences(&text, CAPTURE_WARNING), 1);
    assert!(text.contains("config.yaml MCP server_id(s) cfg-alias are the name or alias"));
    assert_eq!(occurrences(&text, SHADOW_WARNING), 0);
    mutator.reload().await.unwrap();
    assert_eq!(occurrences(&captured.text(), CAPTURE_WARNING), 1);
}

#[tokio::test]
async fn warn_capturing_not_logged_when_db_id_equals_config_id() {
    let fixture = test_fixture();
    fixture
        .put_config_server(server(
            r#"{"server_id":"srv-1","name":"srv-1","transport":"http"}"#,
        ))
        .await;
    fixture.rows.set(vec![row(
        r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","alias":"srv-1"}"#,
    )]);
    let (captured, _guard) = capture();
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.reload().await.unwrap();
    let text = captured.text();
    assert_eq!(occurrences(&text, CAPTURE_WARNING), 0);
    assert_eq!(occurrences(&text, SHADOW_WARNING), 1);
}
