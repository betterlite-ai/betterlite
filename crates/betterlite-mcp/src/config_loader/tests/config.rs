#[tokio::test]
async fn config_defaults_for_a_minimal_entry() {
    let fixture = Fixture::new();
    fixture
        .load(&config(r#"{"test_server": {"url": "https://mcp.test"}}"#))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.name, "test_server");
    assert_eq!(server.fields.server_id.len(), 32);
    assert_eq!(server.fields.server_name.as_deref(), Some("test_server"));
    assert_eq!(server.fields.alias, None);
    assert_eq!(server.fields.url.as_deref(), Some("https://mcp.test"));
    assert_eq!(server.fields.transport, MCPTransport::Http);
    assert_eq!(
        server.fields.protocol_version,
        Some(MCPUpstreamProtocol::Auto)
    );
    assert_eq!(server.fields.auth_type, None);
    assert_eq!(server.fields.command, Some(String::new()));
    assert_eq!(server.fields.args, Some(Vec::new()));
    assert_eq!(server.fields.env, Some(BTreeMap::new()));
    assert!(!server.fields.allow_all_keys);
    assert!(server.fields.available_on_public_internet);
    assert!(!server.fields.delegate_auth_to_upstream);
    assert!(!server.fields.oauth_passthrough);
    assert!(!server.fields.allow_sampling);
    assert!(!server.fields.allow_elicitation);
    assert_eq!(server.fields.dcr_bridge, None);
    assert!(!server.fields.per_server_oauth_discovery);
    assert_eq!(
        server.fields.subject_token_type,
        DEFAULT_SUBJECT_TOKEN_TYPE.to_owned()
    );
    assert_eq!(server.fields.client_assertion_signing_alg, "RS256");
    assert_eq!(server.fields.token_exchange_profile, "rfc8693");
    assert_eq!(server.fields.scopes, None);
    assert_eq!(server.fields.configured_scopes, None);
    assert_eq!(server.fields.issuer, None);
    assert_eq!(server.fields.authorization_url, None);
    assert_eq!(server.fields.token_url, None);
    assert_eq!(server.fields.registration_url, None);
    assert_eq!(server.fields.short_prefix, None);
}

#[tokio::test]
async fn config_reads_transport_and_protocol_version() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "transport": "sse", "protocol_version": "2025-03-26"}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.transport, MCPTransport::Sse);
    assert_eq!(
        server.fields.protocol_version,
        Some(MCPUpstreamProtocol::Mar2025)
    );
}

#[tokio::test]
async fn config_reads_the_catalog_fields() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "instructions": "Use carefully", "extra_headers": ["X-Client"], "allowed_tools": ["a"], "disallowed_tools": ["b"], "allowed_params": {"a": ["p"]}, "static_headers": {"X-Static": "1"}, "access_groups": ["g1"]}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.instructions.as_deref(), Some("Use carefully"));
    assert_eq!(
        server.fields.extra_headers,
        Some(vec![String::from("X-Client")])
    );
    assert_eq!(server.fields.allowed_tools, Some(vec![String::from("a")]));
    assert_eq!(
        server.fields.disallowed_tools,
        Some(vec![String::from("b")])
    );
    let allowed_params = server.fields.allowed_params.unwrap();
    assert_eq!(allowed_params.get("a"), Some(&vec![String::from("p")]));
    let static_headers = server.fields.static_headers.unwrap();
    assert_eq!(
        static_headers.get("X-Static").map(String::as_str),
        Some("1")
    );
    assert_eq!(server.fields.access_groups, Some(vec![String::from("g1")]));
}

#[tokio::test]
async fn config_reads_the_exchange_and_aws_fields() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "token_exchange_endpoint": "https://idp.test/token", "audience": "api", "subject_token_type": "urn:test:subject", "upstream_resource": "https://up.test", "upstream_token_header": "X-Upstream", "id_jag_resource_token_endpoint": "https://idp.test/jag", "id_jag_resource": "https://res.test", "client_private_key": "private-key", "client_private_key_id": "kid-1", "client_assertion_signing_alg": "ES256", "token_exchange_profile": "custom", "aws_access_key_id": "AKIA", "aws_secret_access_key": "secret", "aws_session_token": "session", "aws_region_name": "eu-west-1", "aws_service_name": "execute-api", "aws_role_name": "role", "aws_session_name": "session-name"}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(
        server.fields.token_exchange_endpoint.as_deref(),
        Some("https://idp.test/token")
    );
    assert_eq!(server.fields.audience.as_deref(), Some("api"));
    assert_eq!(server.fields.subject_token_type, "urn:test:subject");
    assert_eq!(
        server.fields.upstream_resource.as_deref(),
        Some("https://up.test")
    );
    assert_eq!(
        server.fields.upstream_token_header.as_deref(),
        Some("X-Upstream")
    );
    assert_eq!(
        server.fields.id_jag_resource_token_endpoint.as_deref(),
        Some("https://idp.test/jag")
    );
    assert_eq!(
        server.fields.id_jag_resource.as_deref(),
        Some("https://res.test")
    );
    assert_eq!(
        secret_text(server.fields.client_private_key.as_ref().unwrap()),
        "private-key"
    );
    assert_eq!(
        server.fields.client_private_key_id.as_deref(),
        Some("kid-1")
    );
    assert_eq!(server.fields.client_assertion_signing_alg, "ES256");
    assert_eq!(server.fields.token_exchange_profile, "custom");
    assert_eq!(
        secret_text(server.fields.aws_access_key_id.as_ref().unwrap()),
        "AKIA"
    );
    assert_eq!(
        secret_text(server.fields.aws_secret_access_key.as_ref().unwrap()),
        "secret"
    );
    assert_eq!(
        secret_text(server.fields.aws_session_token.as_ref().unwrap()),
        "session"
    );
    assert_eq!(server.fields.aws_region_name.as_deref(), Some("eu-west-1"));
    assert_eq!(
        server.fields.aws_service_name.as_deref(),
        Some("execute-api")
    );
    assert_eq!(server.fields.aws_role_name.as_deref(), Some("role"));
    assert_eq!(
        server.fields.aws_session_name.as_deref(),
        Some("session-name")
    );
}

#[tokio::test]
async fn config_reads_the_switches() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "allow_all_keys": true, "available_on_public_internet": false, "delegate_auth_to_upstream": true, "oauth_passthrough": true, "allow_sampling": true, "allow_elicitation": true}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert!(server.fields.allow_all_keys);
    assert!(!server.fields.available_on_public_internet);
    assert!(server.fields.delegate_auth_to_upstream);
    assert!(server.fields.oauth_passthrough);
    assert!(server.fields.allow_sampling);
    assert!(server.fields.allow_elicitation);
}

#[tokio::test]
async fn config_authentication_token_wins_over_auth_value() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_token": {"url": "https://mcp.test", "authentication_token": "tok", "auth_value": "value"}, "test_value": {"url": "https://mcp.test", "auth_value": "value"}, "test_none": {"url": "https://mcp.test"}}"#,
        ))
        .await
        .unwrap();
    let token = fixture.server_named("test_token").await.unwrap();
    assert_eq!(
        secret_text(token.fields.authentication_token.as_ref().unwrap()),
        "tok"
    );
    let value = fixture.server_named("test_value").await.unwrap();
    assert_eq!(
        secret_text(value.fields.authentication_token.as_ref().unwrap()),
        "value"
    );
    let none = fixture.server_named("test_none").await.unwrap();
    assert_eq!(none.fields.authentication_token, None);
}

#[tokio::test]
async fn config_scopes_accept_a_string_and_a_list() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_text": {"url": "https://mcp.test", "scopes": "read write"}, "test_list": {"url": "https://mcp.test", "scopes": ["a", ""]}, "test_blank": {"url": "https://mcp.test", "scopes": [""]}}"#,
        ))
        .await
        .unwrap();
    let text = fixture.server_named("test_text").await.unwrap();
    assert_eq!(
        text.fields.scopes,
        Some(vec![String::from("read"), String::from("write")])
    );
    assert_eq!(text.fields.configured_scopes, text.fields.scopes);
    let list = fixture.server_named("test_list").await.unwrap();
    assert_eq!(list.fields.scopes, Some(vec![String::from("a")]));
    let blank = fixture.server_named("test_blank").await.unwrap();
    assert_eq!(blank.fields.scopes, None);
    assert_eq!(blank.fields.configured_scopes, None);
}

#[tokio::test]
async fn config_mcp_info_defaults_server_name_and_description() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "description": "Top", "mcp_info": {"owner": "team"}}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    let Json::Object(entries) = mcp_info(&server) else {
        panic!("the mcp_info must serialize as an object");
    };
    assert_eq!(
        entries.get("server_name"),
        Some(&Json::Text(String::from("test_server")))
    );
    assert_eq!(
        entries.get("description"),
        Some(&Json::Text(String::from("Top")))
    );
    assert_eq!(
        entries.get("owner"),
        Some(&Json::Text(String::from("team")))
    );
}

#[tokio::test]
async fn config_mcp_info_description_wins_over_the_top_level_one() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "description": "Top", "mcp_info": {"description": "Inner"}}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    let Json::Object(entries) = mcp_info(&server) else {
        panic!("the mcp_info must serialize as an object");
    };
    assert_eq!(
        entries.get("description"),
        Some(&Json::Text(String::from("Inner")))
    );
}

#[tokio::test]
async fn config_mcp_info_skips_an_empty_description() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "description": ""}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    let Json::Object(entries) = mcp_info(&server) else {
        panic!("the mcp_info must serialize as an object");
    };
    assert_eq!(entries.get("description"), None);
}

#[tokio::test]
async fn config_mcp_info_keeps_a_configured_server_name() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "mcp_info": {"server_name": "Custom"}}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    let Json::Object(entries) = mcp_info(&server) else {
        panic!("the mcp_info must serialize as an object");
    };
    assert_eq!(
        entries.get("server_name"),
        Some(&Json::Text(String::from("Custom")))
    );
}

#[tokio::test]
async fn config_env_vars_are_copied() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "env_vars": [{"key": "A", "value": "1"}]}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    let entries = server.fields.env_vars.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries.first().unwrap().get("key"),
        Some(&JsonValue::String(String::from("A")))
    );
}

#[tokio::test]
async fn config_timeout_and_max_concurrent_requests_are_read() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "timeout": 2.5, "max_concurrent_requests": 10}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert_eq!(server.fields.timeout, Some(2.5_f64));
    assert_eq!(server.fields.max_concurrent_requests, Some(10_i64));
}

#[tokio::test]
async fn config_token_validation_object_is_copied() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "token_validation": {"audience": "api"}}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    let validation = server.fields.token_validation.unwrap();
    assert_eq!(
        validation.get("audience"),
        Some(&JsonValue::String(String::from("api")))
    );
}

#[tokio::test]
async fn config_per_server_oauth_discovery_is_recorded() {
    let fixture = Fixture::new();
    fixture
        .load(&config(
            r#"{"test_server": {"url": "https://mcp.test", "auth_type": "oauth2", "oauth2_flow": "authorization_code", "per_server_oauth_discovery": true}}"#,
        ))
        .await
        .unwrap();
    let server = fixture.server_named("test_server").await.unwrap();
    assert!(server.fields.per_server_oauth_discovery);
    assert_eq!(
        server.fields.oauth2_flow,
        Some(OAuth2Flow::AuthorizationCode)
    );
}
