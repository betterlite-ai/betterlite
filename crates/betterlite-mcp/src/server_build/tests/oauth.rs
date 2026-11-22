#[tokio::test]
async fn oauth_deferred_discovery_makes_no_fetch() {
    let harness = harness();
    let (captured, _guard) = capture();
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2","url":"https://mcp.test/mcp"}"#;
    let server = harness.build(&row(source)).await.unwrap();
    assert!(harness.fetcher.urls().is_empty());
    assert!(server.fields.authorization_url.is_none());
    assert!(server.fields.token_url.is_none());
    assert_eq!(harness.slots.entries(), vec![(String::from("srv-1"), true)]);
    assert!(captured.text().is_empty());
}

#[tokio::test]
async fn oauth_startup_discovery_fills_the_endpoints() {
    let mut harness = harness();
    let document = metadata_document("https://issuer.test");
    harness.fetcher = FakeFetcher::answering(&document);
    harness.settings = settings_with_startup(true);
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2","url":"https://mcp.test/mcp"}"#;
    let server = harness.build(&row(source)).await.unwrap();
    let urls = harness.fetcher.urls();
    assert_eq!(urls.len(), 3);
    assert_eq!(urls[0], "https://mcp.test/mcp");
    let authorization = server.fields.authorization_url.as_deref();
    assert_eq!(authorization, Some("https://issuer.test/authorize"));
    let token = server.fields.token_url.as_deref();
    assert_eq!(token, Some("https://issuer.test/token"));
    let registration = server.fields.registration_url.as_deref();
    assert_eq!(registration, Some("https://issuer.test/register"));
    assert_eq!(server.fields.issuer.as_deref(), Some("https://issuer.test"));
    assert!(!server.fields.issuer_is_anchored);
    assert_eq!(
        harness.slots.entries(),
        vec![(String::from("srv-1"), false)]
    );
}

#[tokio::test]
async fn oauth_issuer_anchor_fetches_from_the_issuer() {
    let mut harness = harness();
    let document = metadata_document("https://issuer.test");
    harness.fetcher = FakeFetcher::answering(&document);
    harness.settings = settings_with_startup(true);
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2","url":"https://mcp.test/mcp","issuer":"https://issuer.test","authorization_url":"https://manual.test/authorize","token_url":"https://manual.test/token","registration_url":"https://manual.test/register"}"#;
    let server = harness.build(&row(source)).await.unwrap();
    assert!(server.fields.issuer_is_anchored);
    assert_eq!(server.fields.issuer.as_deref(), Some("https://issuer.test"));
    let authorization = server.fields.authorization_url.as_deref();
    assert_eq!(authorization, Some("https://issuer.test/authorize"));
    let token = server.fields.token_url.as_deref();
    assert_eq!(token, Some("https://issuer.test/token"));
    let registration = server.fields.registration_url.as_deref();
    assert_eq!(registration, Some("https://issuer.test/register"));
    let configured = server.fields.configured_authorization_url.as_deref();
    assert_eq!(configured, Some("https://manual.test/authorize"));
    let configured_token = server.fields.configured_token_url.as_deref();
    assert_eq!(configured_token, Some("https://manual.test/token"));
    assert_eq!(server.fields.scopes, Some(strings(&["read"])));
    assert_eq!(
        harness.slots.entries(),
        vec![(String::from("srv-1"), false)]
    );
}

#[tokio::test]
async fn oauth_issuer_anchor_defers_without_startup_discovery() {
    let mut harness = harness();
    let document = metadata_document("https://issuer.test");
    harness.fetcher = FakeFetcher::answering(&document);
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2","url":"https://mcp.test/mcp","issuer":"https://issuer.test","authorization_url":"https://manual.test/authorize","token_url":"https://manual.test/token"}"#;
    let server = harness.build(&row(source)).await.unwrap();
    assert!(harness.fetcher.urls().is_empty());
    assert!(server.fields.issuer_is_anchored);
    assert_eq!(server.fields.issuer.as_deref(), Some("https://issuer.test"));
    assert!(server.fields.authorization_url.is_none());
    assert!(server.fields.token_url.is_none());
    let configured_token = server.fields.configured_token_url.as_deref();
    assert_eq!(configured_token, Some("https://manual.test/token"));
    assert_eq!(harness.slots.entries(), vec![(String::from("srv-1"), true)]);
}

#[tokio::test]
async fn oauth_gate_drops_uncorroborated_endpoints() {
    let mut harness = harness();
    let document = metadata_document("https://issuer.test");
    harness.fetcher = FakeFetcher::answering(&document);
    harness.settings = settings_with_startup(true);
    let (captured, _guard) = capture();
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2","url":"https://mcp.test/mcp","authorization_url":"https://manual.test/authorize"}"#;
    let server = harness.build(&row(source)).await.unwrap();
    let authorization = server.fields.authorization_url.as_deref();
    assert_eq!(authorization, Some("https://manual.test/authorize"));
    assert!(server.fields.token_url.is_none());
    assert!(server.fields.registration_url.is_none());
    let text = captured.text();
    assert!(text.contains("does not match the manually configured authorization_url"));
    assert!(!text.contains("dcr_bridge"));
    assert_eq!(harness.slots.entries(), vec![(String::from("srv-1"), true)]);
}

#[tokio::test]
async fn oauth_gate_keeps_corroborated_endpoints() {
    let mut harness = harness();
    let document = metadata_document("https://issuer.test");
    harness.fetcher = FakeFetcher::answering(&document);
    harness.settings = settings_with_startup(true);
    let (captured, _guard) = capture();
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2","url":"https://mcp.test/mcp","authorization_url":"https://issuer.test/authorize"}"#;
    let server = harness.build(&row(source)).await.unwrap();
    let token = server.fields.token_url.as_deref();
    assert_eq!(token, Some("https://issuer.test/token"));
    let registration = server.fields.registration_url.as_deref();
    assert_eq!(registration, Some("https://issuer.test/register"));
    assert!(!captured.text().contains("does not match"));
    assert_eq!(
        harness.slots.entries(),
        vec![(String::from("srv-1"), false)]
    );
}

#[tokio::test]
async fn oauth_token_exchange_is_not_gated() {
    let mut harness = harness();
    let document = metadata_document("https://issuer.test");
    harness.fetcher = FakeFetcher::answering(&document);
    harness.settings = settings_with_startup(true);
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2_token_exchange","url":"https://mcp.test/mcp","authorization_url":"https://manual.test/authorize"}"#;
    let server = harness.build(&row(source)).await.unwrap();
    let token = server.fields.token_url.as_deref();
    assert_eq!(token, Some("https://issuer.test/token"));
    let registration = server.fields.registration_url.as_deref();
    assert_eq!(registration, Some("https://issuer.test/register"));
    assert_eq!(server.fields.issuer.as_deref(), Some("https://issuer.test"));
}

#[tokio::test]
async fn oauth_resolve_step_reports_deferred_without_fetching() {
    let mut harness = harness();
    let document = metadata_document("https://issuer.test");
    harness.fetcher = FakeFetcher::answering(&document);
    let mut request = oauth_request();
    request.server_url = Some("https://mcp.test/mcp");
    request.auth_type = Some(MCPAuth::Oauth2);
    request.is_discovery_auth_type = true;
    let (metadata, deferred) = harness
        .builder()
        .resolve_table_oauth_metadata(&request)
        .await;
    assert!(metadata.is_none());
    assert!(deferred);
    assert!(harness.fetcher.urls().is_empty());
    harness.settings = settings_with_startup(true);
    let (startup_metadata, startup_deferred) = harness
        .builder()
        .resolve_table_oauth_metadata(&request)
        .await;
    assert!(startup_metadata.is_some());
    assert!(!startup_deferred);
}

#[tokio::test]
async fn oauth_warns_when_discovery_leaves_the_endpoints_unresolved() {
    let mut harness = harness();
    harness.settings = settings_with_startup(true);
    let (captured, _guard) = capture();
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2","url":"https://mcp.test/mcp"}"#;
    harness.build(&row(source)).await.unwrap();
    let text = captured.text();
    assert!(text.contains("left authorization_url, token_url unresolved"));
    assert!(text.contains("found no authorization server metadata"));
}

#[tokio::test]
async fn oauth_warns_without_a_discovery_source() {
    let harness = harness();
    let (captured, _guard) = capture();
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2"}"#;
    harness.build(&row(source)).await.unwrap();
    let text = captured.text();
    assert!(text.contains("has no discovery source"));
    assert!(text.contains("authorization_url and token_url are not set manually"));
}

#[tokio::test]
async fn oauth_plain_row_does_not_warn_about_missing_endpoints() {
    let harness = harness();
    let (captured, _guard) = capture();
    let source = r#"{"server_id":"srv-1","transport":"http"}"#;
    harness.build(&row(source)).await.unwrap();
    assert!(captured.text().is_empty());
}

#[tokio::test]
async fn oauth_complete_manual_configuration_makes_no_fetch() {
    let mut harness = harness();
    let document = metadata_document("https://issuer.test");
    harness.fetcher = FakeFetcher::answering(&document);
    harness.settings = settings_with_startup(true);
    let (captured, _guard) = capture();
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2","url":"https://mcp.test/mcp","authorization_url":"https://manual.test/authorize","token_url":"https://manual.test/token","credentials":{"scopes":["read"]}}"#;
    let server = harness.build(&row(source)).await.unwrap();
    assert!(harness.fetcher.urls().is_empty());
    assert!(server.fields.registration_url.is_none());
    assert!(captured.text().is_empty());
}

#[tokio::test]
async fn oauth_scopes_and_authorization_url_without_a_token_url_still_discover() {
    let mut harness = harness();
    let document = metadata_document("https://issuer.test");
    harness.fetcher = FakeFetcher::answering(&document);
    harness.settings = settings_with_startup(true);
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2","url":"https://mcp.test/mcp","authorization_url":"https://issuer.test/authorize","credentials":{"scopes":["read"]}}"#;
    let server = harness.build(&row(source)).await.unwrap();
    assert!(!harness.fetcher.urls().is_empty());
    assert_eq!(
        server.fields.token_url.as_deref(),
        Some("https://issuer.test/token")
    );
}

#[tokio::test]
async fn oauth_fetch_ignores_the_issuer_when_the_anchor_is_off() {
    let mut harness = harness();
    let document = metadata_document("https://issuer.test");
    harness.fetcher = FakeFetcher::answering(&document);
    harness.settings = settings_with_startup(true);
    let mut request = oauth_request();
    request.server_url = Some("https://mcp.test/mcp");
    request.auth_type = Some(MCPAuth::Oauth2);
    request.is_discovery_auth_type = true;
    request.manual_issuer = Some("https://issuer.test");
    let (metadata, deferred) = harness
        .builder()
        .resolve_table_oauth_metadata(&request)
        .await;
    assert!(metadata.is_some());
    assert!(!deferred);
    let urls = harness.fetcher.urls();
    assert!(urls[0].starts_with("https://mcp.test/mcp"));
}
