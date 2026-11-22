async fn built(source: &str) -> MCPServer {
    let harness = harness();
    let table = row(source);
    harness.build(&table).await.unwrap()
}

#[tokio::test]
async fn fields_record_defaults_match_the_python_record() {
    let server = built(r#"{"server_id":"srv-1","transport":"http"}"#).await;
    assert_eq!(server.fields.name, "srv-1");
    let protocol = server.fields.protocol_version;
    assert_eq!(protocol, Some(MCPUpstreamProtocol::Auto));
    assert_eq!(server.fields.subject_token_type, DEFAULT_SUBJECT_TOKEN_TYPE);
    assert_eq!(server.fields.client_assertion_signing_alg, "RS256");
    assert_eq!(server.fields.token_exchange_profile, "rfc8693");
    assert!(server.fields.available_on_public_internet);
    assert!(!server.fields.allow_all_keys);
    assert!(!server.fields.delegate_auth_to_upstream);
    assert!(!server.fields.oauth_passthrough);
    assert!(!server.fields.per_server_oauth_discovery);
    assert!(!server.fields.is_byok);
    assert_eq!(server.fields.allowed_tools, Some(Vec::new()));
    assert_eq!(server.fields.extra_headers, Some(Vec::new()));
    assert_eq!(server.fields.access_groups, Some(Vec::new()));
    assert_eq!(server.fields.args, Some(Vec::new()));
    assert!(server.fields.env.is_none());
    assert!(server.fields.static_headers.is_none());
    assert!(server.fields.env_vars.is_none());
    assert!(server.fields.oauth2_flow.is_none());
    assert!(server.fields.created_at.is_none());
    assert!(server.fields.byok_description.is_empty());
    assert!(server.fields.short_prefix.is_none());
}

#[tokio::test]
async fn fields_row_flags_override_the_defaults() {
    let source = r#"{"server_id":"srv-1","transport":"http","allow_all_keys":true,"available_on_public_internet":false,"delegate_auth_to_upstream":true,"oauth_passthrough":true,"per_server_oauth_discovery":true,"is_byok":true}"#;
    let server = built(source).await;
    assert!(!server.fields.available_on_public_internet);
    assert!(server.fields.allow_all_keys);
    assert!(server.fields.delegate_auth_to_upstream);
    assert!(server.fields.oauth_passthrough);
    assert!(server.fields.per_server_oauth_discovery);
    assert!(server.fields.is_byok);
}

#[tokio::test]
async fn fields_name_prefers_alias_then_server_name_then_id() {
    let aliased = r#"{"server_id":"srv-1","transport":"http","alias":"test-alias","server_name":"test-name"}"#;
    assert_eq!(built(aliased).await.fields.name, "test-alias");
    let named = r#"{"server_id":"srv-1","transport":"http","server_name":"test-name"}"#;
    assert_eq!(built(named).await.fields.name, "test-name");
    let bare = r#"{"server_id":"srv-1","transport":"http"}"#;
    assert_eq!(built(bare).await.fields.name, "srv-1");
}

#[tokio::test]
async fn fields_columns_win_over_the_credentials_blob() {
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2_token_exchange","token_exchange_endpoint":"https://columns.test/token","audience":"columns-audience","subject_token_type":"columns-subject","token_exchange_profile":"columns-profile","credentials":{"token_exchange_endpoint":"https://blob.test/token","audience":"blob-audience","subject_token_type":"blob-subject","token_exchange_profile":"blob-profile"}}"#;
    let server = built(source).await;
    let endpoint = server.fields.token_exchange_endpoint.as_deref();
    assert_eq!(endpoint, Some("https://columns.test/token"));
    assert_eq!(server.fields.audience.as_deref(), Some("columns-audience"));
    assert_eq!(server.fields.subject_token_type, "columns-subject");
    assert_eq!(server.fields.token_exchange_profile, "columns-profile");
}

#[tokio::test]
async fn fields_blob_fills_the_missing_columns() {
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2_token_exchange","credentials":{"token_exchange_endpoint":"https://blob.test/token","audience":"blob-audience","subject_token_type":"blob-subject","token_exchange_profile":"blob-profile"}}"#;
    let server = built(source).await;
    let endpoint = server.fields.token_exchange_endpoint.as_deref();
    assert_eq!(endpoint, Some("https://blob.test/token"));
    assert_eq!(server.fields.audience.as_deref(), Some("blob-audience"));
    assert_eq!(server.fields.subject_token_type, "blob-subject");
    assert_eq!(server.fields.token_exchange_profile, "blob-profile");
}

#[tokio::test]
async fn fields_configured_scopes_need_a_non_empty_list_of_strings() {
    let listed =
        r#"{"server_id":"srv-1","transport":"http","credentials":{"scopes":["read","write"]}}"#;
    let server = built(listed).await;
    assert_eq!(server.fields.scopes, Some(strings(&["read", "write"])));
    assert_eq!(
        server.fields.configured_scopes,
        Some(strings(&["read", "write"]))
    );
    let blank = r#"{"server_id":"srv-1","transport":"http","credentials":{"scopes":["read",""]}}"#;
    let filtered = built(blank).await;
    assert_eq!(filtered.fields.scopes, Some(strings(&["read"])));
    assert!(filtered.fields.configured_scopes.is_none());
    let empty = r#"{"server_id":"srv-1","transport":"http","credentials":{"scopes":[]}}"#;
    let absent = built(empty).await;
    assert!(absent.fields.scopes.is_none());
    assert!(absent.fields.configured_scopes.is_none());
}

#[tokio::test]
async fn fields_scopes_fall_back_to_the_gated_metadata() {
    let mut harness = harness();
    let document = metadata_document("https://issuer.test");
    harness.fetcher = FakeFetcher::answering(&document);
    harness.settings = settings_with_startup(true);
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2","url":"https://mcp.test/mcp"}"#;
    let server = harness.build(&row(source)).await.unwrap();
    assert_eq!(server.fields.scopes, Some(strings(&["read"])));
    assert!(server.fields.configured_scopes.is_none());
}

#[tokio::test]
async fn fields_mcp_info_defaults_come_from_the_row() {
    let named = r#"{"server_id":"srv-1","transport":"http","server_name":"test-name","description":"test description"}"#;
    let info = serde_json::to_string(&built(named).await.fields.mcp_info).unwrap();
    assert_eq!(
        info,
        r#"{"description":"test description","server_name":"test-name"}"#
    );
    let bare = r#"{"server_id":"srv-1","transport":"http","description":""}"#;
    let fallback = serde_json::to_string(&built(bare).await.fields.mcp_info).unwrap();
    assert_eq!(fallback, r#"{"server_name":"srv-1"}"#);
}

#[tokio::test]
async fn fields_mcp_info_keeps_explicit_values() {
    let source = r#"{"server_id":"srv-1","transport":"http","server_name":"test-name","description":"test description","mcp_info":{"server_name":"kept-name","description":"kept description"}}"#;
    let server = built(source).await;
    let info = serde_json::to_string(&server.fields.mcp_info).unwrap();
    assert_eq!(
        info,
        r#"{"description":"kept description","server_name":"kept-name"}"#
    );
}

#[tokio::test]
async fn fields_mcp_info_costs_are_normalized() {
    let source = r#"{"server_id":"srv-1","transport":"http","mcp_info":{"server_name":"test-name","mcp_server_cost_info":{"default_cost_per_query":"0.5","tool_name_to_cost_per_query":{"tool_a":0.25,"tool_b":"1.5"}}}}"#;
    let server = built(source).await;
    let info = serde_json::to_string(&server.fields.mcp_info).unwrap();
    assert_eq!(
        info,
        r#"{"mcp_server_cost_info":{"default_cost_per_query":0.5,"tool_name_to_cost_per_query":{"tool_a":0.25,"tool_b":1.5}},"server_name":"test-name"}"#
    );
}

#[tokio::test]
async fn fields_env_and_static_headers_are_exposed() {
    let source = r#"{"server_id":"srv-1","transport":"stdio","command":"npx","env":{"TOKEN":"test-env"},"static_headers":{"X-Test":"test-header"}}"#;
    let server = built(source).await;
    let env = BTreeMap::from([(String::from("TOKEN"), String::from("test-env"))]);
    assert_eq!(server.fields.env, Some(env));
    let headers = BTreeMap::from([(String::from("X-Test"), String::from("test-header"))]);
    assert_eq!(server.fields.static_headers, Some(headers));
    assert_eq!(server.fields.command.as_deref(), Some("npx"));
}

#[tokio::test]
async fn fields_timestamps_are_normalized_to_utc() {
    let source = r#"{"server_id":"srv-1","transport":"http","created_at":"2024-01-02T03:04:05+02:00","updated_at":"2024-01-02 03:04:05"}"#;
    let server = built(source).await;
    let created = server.fields.created_at.unwrap().to_rfc3339();
    assert_eq!(created, "2024-01-02T01:04:05+00:00");
    let updated = server.fields.updated_at.unwrap().to_rfc3339();
    assert_eq!(updated, "2024-01-02T03:04:05+00:00");
}

#[tokio::test]
async fn fields_timeout_and_lists_carry_over() {
    let source = r#"{"server_id":"srv-1","transport":"http","timeout":12.5,"max_concurrent_requests":4,"args":["--port","8080"],"extra_headers":["x-test"],"mcp_access_groups":["group-a"],"allowed_tools":["tool-a"]}"#;
    let server = built(source).await;
    assert_eq!(server.fields.timeout, Some(12.5));
    assert_eq!(server.fields.max_concurrent_requests, Some(4));
    assert_eq!(server.fields.args, Some(strings(&["--port", "8080"])));
    assert_eq!(server.fields.extra_headers, Some(strings(&["x-test"])));
    assert_eq!(server.fields.access_groups, Some(strings(&["group-a"])));
    assert_eq!(server.fields.allowed_tools, Some(strings(&["tool-a"])));
}

#[tokio::test]
async fn fields_oauth2_flow_is_explicit_only() {
    let shaped = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2","token_url":"https://issuer.test/token","credentials":{"client_id":"test-id","client_secret":"test-secret"}}"#;
    assert!(built(shaped).await.fields.oauth2_flow.is_none());
    let explicit = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2","oauth2_flow":"client_credentials","token_url":"https://issuer.test/token","credentials":{"client_id":"test-id","client_secret":"test-secret"}}"#;
    let server = built(explicit).await;
    assert_eq!(
        server.fields.oauth2_flow,
        Some(OAuth2Flow::ClientCredentials)
    );
}
