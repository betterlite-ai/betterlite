#[tokio::test(start_paused = true)]
async fn state_without_a_slot_returns_the_registered_server_without_a_fetch() {
    let registered = server("alpha", "oauth2");
    let harness = build_harness(
        ScriptedFetcher::discoverable(vec![EMPTY_DOCUMENT]),
        registry_of(Vec::new(), vec![registered.clone()]),
        DiscoverySettings::default(),
    );
    let mut stale_copy = registered.clone();
    stale_copy.fields.alias = Some(String::from("stale"));
    let result = harness.slots.ensure_discovered(&stale_copy).await;
    assert_eq!(result.unwrap(), registered);
    assert!(harness.fetcher.called_urls().is_empty());
}

#[tokio::test(start_paused = true)]
async fn state_resolution_publishes_and_later_calls_make_no_fetch() {
    let alpha = server("alpha", "oauth2");
    let harness = build_harness(
        ScriptedFetcher::discoverable(vec![EMPTY_DOCUMENT]),
        registry_of(Vec::new(), vec![alpha.clone()]),
        DiscoverySettings::default(),
    );
    harness.slots.set_deferred("alpha", true);
    let first = harness.slots.ensure_discovered(&alpha).await.unwrap();
    assert_eq!(first, discovered(alpha.clone()));
    assert_eq!(registered_db(&harness, "alpha").await, first);
    assert!(!harness.slots.slot_exists("alpha"));
    let second = harness.slots.ensure_discovered(&alpha).await.unwrap();
    assert_eq!(second, first);
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 1);
}

#[tokio::test(start_paused = true)]
async fn state_failed_slot_inside_its_backoff_fails_without_a_fetch() {
    let alpha = server("alpha", "oauth2");
    let harness = build_harness(
        ScriptedFetcher::new(vec![(SERVER_URL, vec![EMPTY_DOCUMENT])]),
        registry_of(Vec::new(), vec![alpha.clone()]),
        DiscoverySettings::default(),
    );
    harness.slots.set_deferred("alpha", true);
    assert_eq!(
        harness.slots.ensure_discovered(&alpha).await,
        incomplete("alpha")
    );
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 1);
    harness.clock.advance(Duration::from_millis(29_999));
    assert_eq!(
        harness.slots.ensure_discovered(&alpha).await,
        incomplete("alpha")
    );
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 1);
    harness.clock.advance(Duration::from_millis(1));
    assert_eq!(
        harness.slots.ensure_discovered(&alpha).await,
        incomplete("alpha")
    );
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 2);
    harness.clock.advance(Duration::from_millis(59_999));
    assert_eq!(
        harness.slots.ensure_discovered(&alpha).await,
        incomplete("alpha")
    );
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 2);
    harness.clock.advance(Duration::from_millis(1));
    assert_eq!(
        harness.slots.ensure_discovered(&alpha).await,
        incomplete("alpha")
    );
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 3);
}

#[rstest]
#[case::alias(Some("Docs"), Some("docs-server"), "Docs")]
#[case::server_name(None, Some("docs-server"), "docs-server")]
#[case::name(None, None, "alpha")]
#[tokio::test(start_paused = true)]
async fn state_unavailable_without_a_client_token_names_the_server(
    #[case] alias: Option<&str>,
    #[case] server_name: Option<&str>,
    #[case] expected: &str,
) {
    let mut alpha = server("alpha", "oauth2");
    alpha.fields.alias = alias.map(str::to_owned);
    alpha.fields.server_name = server_name.map(str::to_owned);
    let harness = build_harness(
        ScriptedFetcher::new(vec![(SERVER_URL, vec![EMPTY_DOCUMENT])]),
        registry_of(Vec::new(), vec![alpha.clone()]),
        DiscoverySettings::default(),
    );
    harness.slots.set_deferred("alpha", true);
    let error = harness.slots.ensure_discovered(&alpha).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "OAuth metadata discovery returned incomplete metadata for MCP server '{expected}'"
        )
    );
    assert_eq!(Err(error), incomplete(expected));
}

#[tokio::test(start_paused = true)]
async fn state_client_token_returns_the_unchanged_server_and_records_the_failure() {
    let passthrough = server("alpha", "true_passthrough");
    let harness = build_harness(
        ScriptedFetcher::new(vec![(SERVER_URL, vec![EMPTY_DOCUMENT])]),
        registry_of(Vec::new(), vec![passthrough.clone()]),
        DiscoverySettings::default(),
    );
    harness.slots.set_deferred("alpha", true);
    let first = harness.slots.ensure_discovered(&passthrough).await;
    assert_eq!(first.unwrap(), passthrough);
    assert!(harness.slots.slot_exists("alpha"));
    let second = harness.slots.ensure_discovered(&passthrough).await;
    assert_eq!(second.unwrap(), passthrough);
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 1);
}

#[tokio::test(start_paused = true)]
async fn state_temporary_slot_lives_for_the_ttl() {
    let draft = server("draft", "oauth2");
    let harness = build_harness(
        ScriptedFetcher::discoverable(vec![EMPTY_DOCUMENT]),
        registry_of(Vec::new(), Vec::new()),
        DiscoverySettings::default(),
    );
    harness.slots.set_deferred("draft", true);
    let first = harness.slots.ensure_discovered(&draft).await.unwrap();
    assert_eq!(first, discovered(draft.clone()));
    assert!(harness.slots.slot_exists("draft"));
    let second = harness.slots.ensure_discovered(&draft).await.unwrap();
    assert_eq!(second, first);
    harness.clock.advance(Duration::from_millis(299_999));
    assert!(harness.slots.slot_exists("draft"));
    harness.clock.advance(Duration::from_millis(1));
    assert!(!harness.slots.slot_exists("draft"));
    let expired = harness.slots.ensure_discovered(&draft).await.unwrap();
    assert_eq!(expired, draft);
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 1);
}

#[tokio::test(start_paused = true)]
async fn state_publish_writes_a_config_server_into_the_config_entry() {
    let alpha = server("alpha", "oauth2");
    let harness = build_harness(
        ScriptedFetcher::discoverable(vec![EMPTY_DOCUMENT]),
        registry_of(vec![alpha.clone()], Vec::new()),
        DiscoverySettings::default(),
    );
    harness.slots.set_deferred("alpha", true);
    let resolved = harness.slots.ensure_discovered(&alpha).await.unwrap();
    assert_eq!(resolved, discovered(alpha.clone()));
    let registry = harness.registry.read().await;
    assert_eq!(registry.config_servers.get("alpha"), Some(&resolved));
    assert!(registry.db_servers.is_empty());
    drop(registry);
    assert!(!harness.slots.slot_exists("alpha"));
}

#[tokio::test(start_paused = true)]
async fn state_publish_prefers_the_database_entry() {
    let mut from_config = server("alpha", "oauth2");
    from_config.fields.alias = Some(String::from("config"));
    let mut from_database = server("alpha", "oauth2");
    from_database.fields.alias = Some(String::from("database"));
    let harness = build_harness(
        ScriptedFetcher::discoverable(vec![EMPTY_DOCUMENT]),
        registry_of(vec![from_config.clone()], vec![from_database.clone()]),
        DiscoverySettings::default(),
    );
    harness.slots.set_deferred("alpha", true);
    let resolved = harness.slots.ensure_discovered(&from_config).await.unwrap();
    assert_eq!(resolved, discovered(from_database));
    let registry = harness.registry.read().await;
    assert_eq!(registry.db_servers.get("alpha"), Some(&resolved));
    assert_eq!(registry.config_servers.get("alpha"), Some(&from_config));
}

#[tokio::test(start_paused = true)]
async fn state_resolved_registered_server_publishes_without_a_fetch() {
    let alpha = with_endpoints(server("alpha", "oauth2"));
    let harness = build_harness(
        ScriptedFetcher::discoverable(vec![EMPTY_DOCUMENT]),
        registry_of(Vec::new(), vec![alpha.clone()]),
        DiscoverySettings::default(),
    );
    harness.slots.set_deferred("alpha", true);
    let resolved = harness.slots.ensure_discovered(&alpha).await.unwrap();
    assert_eq!(resolved, alpha);
    assert!(harness.fetcher.called_urls().is_empty());
    assert!(!harness.slots.slot_exists("alpha"));
}

#[tokio::test(start_paused = true)]
async fn state_registered_server_reads_database_then_config_then_the_given_server() {
    let mut database_alpha = server("alpha", "oauth2");
    database_alpha.fields.alias = Some(String::from("database"));
    let mut config_alpha = server("alpha", "oauth2");
    config_alpha.fields.alias = Some(String::from("config"));
    let mut config_beta = server("beta", "oauth2");
    config_beta.fields.alias = Some(String::from("config"));
    let harness = build_harness(
        ScriptedFetcher::default(),
        registry_of(
            vec![config_alpha, config_beta.clone()],
            vec![database_alpha.clone()],
        ),
        DiscoverySettings::default(),
    );
    let slots = &harness.slots;
    assert_eq!(
        slots.registered_server(&server("alpha", "oauth2")).await,
        database_alpha
    );
    assert_eq!(
        slots.registered_server(&server("beta", "oauth2")).await,
        config_beta
    );
    let gamma = server("gamma", "oauth2");
    assert_eq!(slots.registered_server(&gamma).await, gamma);
}

#[tokio::test(start_paused = true)]
async fn state_tokio_clock_follows_the_runtime_clock() {
    let before = TokioClock.now();
    advance(Duration::from_secs(3)).await;
    assert_eq!(
        TokioClock.now().duration_since(before),
        Duration::from_secs(3)
    );
}
