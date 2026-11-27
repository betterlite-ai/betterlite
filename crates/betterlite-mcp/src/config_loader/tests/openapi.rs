#[tokio::test]
async fn openapi_registers_every_operation_with_prefixed_names() {
    let fixture = Fixture::new();
    let spec = corpus("petstore.json");
    let source =
        format!(r#"{{"test_server": {{"url": "https://api.test", "spec_path": "{spec}"}}}}"#);
    fixture.load(&config(&source)).await.unwrap();
    let mut names = fixture.tools.names();
    names.sort();
    assert_eq!(
        names,
        vec![
            String::from("test_server-createpet"),
            String::from("test_server-deletepet"),
            String::from("test_server-get.pet_by_id"),
            String::from("test_server-listpets"),
        ]
    );
    let descriptions = fixture.tools.descriptions();
    assert_eq!(
        descriptions.get("test_server-listpets").map(String::as_str),
        Some("List pets")
    );
    assert_eq!(
        descriptions
            .get("test_server-createpet")
            .map(String::as_str),
        Some("Create a pet")
    );
    assert_eq!(
        descriptions
            .get("test_server-get.pet_by_id")
            .map(String::as_str),
        Some("GET /pets/{petId}")
    );
}

#[tokio::test]
async fn openapi_falls_back_to_method_and_path() {
    let fixture = Fixture::new();
    let spec = corpus("missing-operation-id.json");
    let source =
        format!(r#"{{"test_server": {{"url": "https://anon.test", "spec_path": "{spec}"}}}}"#);
    fixture.load(&config(&source)).await.unwrap();
    let names = fixture.tools.names();
    assert!(names.contains(&String::from("test_server-get__items")));
    assert!(names.contains(&String::from("test_server-post__items")));
    assert!(names.contains(&String::from("test_server-delete__items_{itemid}")));
    let descriptions = fixture.tools.descriptions();
    assert_eq!(
        descriptions
            .get("test_server-get__items")
            .map(String::as_str),
        Some("List items")
    );
    assert_eq!(
        descriptions
            .get("test_server-post__items")
            .map(String::as_str),
        Some("POST /items")
    );
    assert_eq!(
        descriptions
            .get("test_server-delete__items_{itemid}")
            .map(String::as_str),
        Some("Remove an item")
    );
}

#[tokio::test]
async fn openapi_tool_owners_include_base_and_prefixed_names() {
    let fixture = Fixture::new();
    let spec = corpus("petstore.json");
    let source =
        format!(r#"{{"test_server": {{"url": "https://api.test", "spec_path": "{spec}"}}}}"#);
    fixture.load(&config(&source)).await.unwrap();
    let owners = fixture.tools.owners();
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
async fn openapi_authorization_header_follows_the_auth_type() {
    let fixture = Fixture::new();
    let spec = corpus("petstore.json");
    let source = format!(
        r#"{{"test_bearer": {{"url": "https://api.test", "spec_path": "{spec}", "auth_type": "bearer_token", "authentication_token": "Bearer abc"}}, "test_apikey": {{"url": "https://api.test", "spec_path": "{spec}", "auth_type": "api_key", "authentication_token": "ApiKey key"}}, "test_basic": {{"url": "https://api.test", "spec_path": "{spec}", "auth_type": "basic", "authentication_token": "user:pass"}}, "test_token": {{"url": "https://api.test", "spec_path": "{spec}", "auth_type": "token", "authentication_token": "token t"}}}}"#
    );
    fixture.load(&config(&source)).await.unwrap();
    let header = |name: &str| {
        fixture
            .tools
            .config_for(name)
            .headers
            .get("Authorization")
            .cloned()
    };
    assert_eq!(
        header("test_bearer-listpets"),
        Some(String::from("Bearer abc"))
    );
    assert_eq!(
        fixture.tools.config_for("test_bearer-listpets").auth_type,
        Some(McpAuthType::BearerToken)
    );
    assert_eq!(
        header("test_apikey-listpets"),
        Some(String::from("ApiKey key"))
    );
    assert_eq!(
        header("test_basic-listpets"),
        Some(String::from("Basic dXNlcjpwYXNz"))
    );
    assert_eq!(header("test_token-listpets"), Some(String::from("token t")));
}

#[tokio::test]
async fn openapi_static_headers_override_the_authorization_header() {
    let fixture = Fixture::new();
    let spec = corpus("petstore.json");
    let source = format!(
        r#"{{"test_server": {{"url": "https://api.test", "spec_path": "{spec}", "auth_type": "bearer_token", "authentication_token": "Bearer abc", "static_headers": {{"Authorization": "Static", "X-Static": "1"}}}}}}"#
    );
    fixture.load(&config(&source)).await.unwrap();
    let headers = fixture.tools.config_for("test_server-listpets").headers;
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Static")
    );
    assert_eq!(headers.get("X-Static").map(String::as_str), Some("1"));
}

#[tokio::test]
async fn openapi_the_configured_url_is_the_base_url() {
    let fixture = Fixture::new();
    let spec = corpus("petstore.json");
    let source =
        format!(r#"{{"test_server": {{"url": "https://api.test", "spec_path": "{spec}"}}}}"#);
    fixture.load(&config(&source)).await.unwrap();
    let tool = fixture.tools.config_for("test_server-listpets");
    assert_eq!(tool.base_url, "https://api.test");
    assert_eq!(tool.path, "/pets");
    assert_eq!(tool.method, "get");
    assert_eq!(tool.server_label.as_deref(), Some("test_server"));
    assert!(!tool.relays_upstream_auth);
    assert_eq!(tool.auth_type, None);
}

#[tokio::test]
async fn openapi_the_base_url_falls_back_to_the_spec_servers() {
    let fixture = Fixture::new();
    let spec = corpus("petstore.json");
    let source = format!(r#"{{"test_server": {{"spec_path": "{spec}"}}}}"#);
    fixture.load(&config(&source)).await.unwrap();
    let tool = fixture.tools.config_for("test_server-listpets");
    assert_eq!(tool.base_url, "https://petstore.test/v1");
}

#[tokio::test]
async fn openapi_a_missing_spec_fails_the_load_with_a_logged_error() {
    let fixture = Fixture::new();
    let (captured, _guard) = capture();
    let error = fixture
        .load(&config(
            r#"{"test_server": {"url": "https://api.test", "spec_path": "/nonexistent/b6764-spec.json"}}"#,
        ))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("Failed to register OpenAPI tools for server test_server: ")
    );
    assert!(
        captured
            .text()
            .contains("Failed to register OpenAPI tools for server test_server:")
    );
}

#[tokio::test]
async fn openapi_tools_use_the_short_prefix_when_enabled() {
    let mut fixture = Fixture::new();
    fixture.prefixes.short_prefix_enabled = true;
    let spec = corpus("petstore.json");
    let source =
        format!(r#"{{"test_server": {{"url": "https://api.test", "spec_path": "{spec}"}}}}"#);
    fixture.load(&config(&source)).await.unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    let prefix = compute_short_server_prefix(&server.fields.server_id, 0).unwrap();
    let names = fixture.tools.names();
    assert!(!names.is_empty());
    for name in names {
        assert!(name.starts_with(&format!("{prefix}-")));
    }
}

#[tokio::test]
async fn openapi_sensitive_header_values_are_not_logged() {
    let fixture = Fixture::new();
    let spec = corpus("petstore.json");
    let source = format!(
        r#"{{"test_server": {{"url": "https://api.test", "spec_path": "{spec}", "auth_type": "bearer_token", "authentication_token": "Bearer abc"}}}}"#
    );
    let (captured, _guard) = capture();
    fixture.load(&config(&source)).await.unwrap();
    let text = captured.text();
    assert!(text.contains(
        "Using headers for OpenAPI tools (excluding sensitive values): ['Authorization']"
    ));
    assert!(!text.contains("Bearer abc"));
}
