fn gated_harness(server: &MCPServer, gate: &Arc<Semaphore>) -> Harness {
    build_harness(
        ScriptedFetcher::discoverable(vec![EMPTY_DOCUMENT]).with_gate(gate),
        registry_of(Vec::new(), vec![server.clone()]),
        DiscoverySettings::default(),
    )
}

#[tokio::test(start_paused = true)]
async fn concurrency_burst_of_callers_makes_one_fetch() {
    let alpha = server("alpha", "oauth2");
    let gate = Arc::new(Semaphore::new(0));
    let harness = gated_harness(&alpha, &gate);
    harness.slots.set_deferred("alpha", true);
    let release = async {
        wait_for_calls(&harness.fetcher, SERVER_URL, 1).await;
        yield_now().await;
        gate.add_permits(1);
    };
    let (first, second, third, ()) = tokio::join!(
        harness.slots.ensure_discovered(&alpha),
        harness.slots.ensure_discovered(&alpha),
        harness.slots.ensure_discovered(&alpha),
        release
    );
    let expected = discovered(alpha);
    assert_eq!(first.unwrap(), expected);
    assert_eq!(second.unwrap(), expected);
    assert_eq!(third.unwrap(), expected);
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 1);
}

#[tokio::test(start_paused = true)]
async fn concurrency_publish_leaves_the_registry_writable_during_a_fetch() {
    let alpha = server("alpha", "oauth2");
    let gate = Arc::new(Semaphore::new(0));
    let harness = gated_harness(&alpha, &gate);
    harness.slots.set_deferred("alpha", true);
    harness.slots.prime(&[alpha.clone()]);
    wait_for_calls(&harness.fetcher, SERVER_URL, 1).await;
    drop(harness.registry.try_write().unwrap());
    gate.add_permits(1);
    let resolved = harness.slots.ensure_discovered(&alpha).await.unwrap();
    assert_eq!(resolved, discovered(alpha));
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 1);
}

#[rstest]
#[case::flag_off(false)]
#[case::flag_on(true)]
#[tokio::test(start_paused = true)]
async fn concurrency_prime_starts_only_the_deferred_servers(#[case] on_startup: bool) {
    let alpha = server("alpha", "oauth2");
    let mut beta = server("beta", "oauth2");
    beta.fields.url = Some(String::from(MOVED_URL));
    let harness = build_harness(
        ScriptedFetcher::discoverable(vec![EMPTY_DOCUMENT]),
        registry_of(Vec::new(), vec![alpha.clone(), beta.clone()]),
        settings_with(on_startup, litellm_delays()),
    );
    harness.slots.set_deferred("alpha", true);
    harness.slots.prime(&[alpha.clone(), beta.clone()]);
    sleep(Duration::from_secs(1)).await;
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 1);
    assert_eq!(harness.fetcher.calls_to(MOVED_URL), 0);
    assert_eq!(registered_db(&harness, "alpha").await, discovered(alpha));
    assert_eq!(registered_db(&harness, "beta").await, beta);
}

#[tokio::test(start_paused = true)]
async fn concurrency_reconcile_adds_and_drops_slots() {
    let alpha = server("alpha", "oauth2");
    let beta = with_endpoints(server("beta", "oauth2"));
    let gamma = with_endpoints(server("gamma", "oauth2"));
    let harness = build_harness(
        ScriptedFetcher::default(),
        registry_of(Vec::new(), vec![alpha.clone(), beta.clone(), gamma.clone()]),
        DiscoverySettings::default(),
    );
    harness.slots.set_deferred("beta", true);
    harness.slots.reconcile(&[alpha, beta, gamma]);
    assert!(harness.slots.slot_exists("alpha"));
    assert!(!harness.slots.slot_exists("beta"));
    assert!(!harness.slots.slot_exists("gamma"));
    assert!(harness.fetcher.called_urls().is_empty());
}

#[tokio::test(start_paused = true)]
async fn concurrency_reconcile_keeps_the_run_of_an_unchanged_server() {
    let alpha = server("alpha", "oauth2");
    let gate = Arc::new(Semaphore::new(0));
    let harness = gated_harness(&alpha, &gate);
    harness.slots.set_deferred("alpha", true);
    harness.slots.prime(&[alpha.clone()]);
    wait_for_calls(&harness.fetcher, SERVER_URL, 1).await;
    harness.slots.reconcile(&[alpha.clone()]);
    gate.add_permits(1);
    let resolved = harness.slots.ensure_discovered(&alpha).await.unwrap();
    assert_eq!(resolved, discovered(alpha));
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 1);
}

#[rstest]
#[case::url_changed(MOVED_URL, "oauth2")]
#[case::auth_type_changed(SERVER_URL, "true_passthrough")]
#[tokio::test(start_paused = true)]
async fn concurrency_reconcile_drops_the_run_of_a_changed_server(
    #[case] url: &str,
    #[case] auth_type: &str,
) {
    let alpha = server("alpha", "oauth2");
    let gate = Arc::new(Semaphore::new(0));
    let harness = gated_harness(&alpha, &gate);
    harness.slots.set_deferred("alpha", true);
    harness.slots.prime(&[alpha.clone()]);
    wait_for_calls(&harness.fetcher, SERVER_URL, 1).await;
    let mut changed = server("alpha", auth_type);
    changed.fields.url = Some(url.to_owned());
    harness
        .registry
        .write()
        .await
        .db_servers
        .insert(String::from("alpha"), changed.clone());
    harness.slots.reconcile(&[changed.clone()]);
    assert!(harness.slots.slot_exists("alpha"));
    gate.add_permits(1);
    sleep(Duration::from_secs(1)).await;
    assert_eq!(registered_db(&harness, "alpha").await, changed);
}

#[tokio::test(start_paused = true)]
async fn concurrency_invalidate_drops_the_slot_and_cancels_the_run() {
    let alpha = server("alpha", "oauth2");
    let gate = Arc::new(Semaphore::new(0));
    let harness = gated_harness(&alpha, &gate);
    harness.slots.set_deferred("alpha", true);
    harness.slots.prime(&[alpha.clone()]);
    wait_for_calls(&harness.fetcher, SERVER_URL, 1).await;
    harness.slots.invalidate_state("alpha");
    assert!(!harness.slots.slot_exists("alpha"));
    gate.add_permits(1);
    sleep(Duration::from_secs(1)).await;
    assert_eq!(registered_db(&harness, "alpha").await, alpha);
    assert_eq!(
        harness.slots.ensure_discovered(&alpha).await.unwrap(),
        alpha
    );
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 1);
}

#[tokio::test(start_paused = true)]
async fn concurrency_a_waiting_caller_rejoins_a_replaced_slot() {
    let alpha = server("alpha", "oauth2");
    let gate = Arc::new(Semaphore::new(0));
    let harness = gated_harness(&alpha, &gate);
    harness.slots.set_deferred("alpha", true);
    let replace = async {
        wait_for_calls(&harness.fetcher, SERVER_URL, 1).await;
        harness.slots.set_deferred("alpha", true);
        gate.add_permits(1);
    };
    let (result, ()) = tokio::join!(harness.slots.ensure_discovered(&alpha), replace);
    assert_eq!(result.unwrap(), discovered(alpha));
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 2);
}

#[rstest]
#[case::gateway_managed("oauth2", false, Some(DiscoveryUnavailable::ChangedRepeatedly))]
#[case::client_forwarded("true_passthrough", false, None)]
#[case::resolved_meanwhile("oauth2", true, None)]
#[tokio::test(start_paused = true)]
async fn concurrency_a_slot_replaced_twice_answers_changed_repeatedly(
    #[case] auth_type: &str,
    #[case] resolved_meanwhile: bool,
    #[case] expected_error: Option<DiscoveryUnavailable>,
) {
    let alpha = server("alpha", auth_type);
    let gate = Arc::new(Semaphore::new(0));
    let harness = gated_harness(&alpha, &gate);
    harness.slots.set_deferred("alpha", true);
    let replace_twice = async {
        wait_for_calls(&harness.fetcher, SERVER_URL, 1).await;
        harness.slots.set_deferred("alpha", true);
        wait_for_calls(&harness.fetcher, SERVER_URL, 2).await;
        if resolved_meanwhile {
            harness
                .registry
                .write()
                .await
                .db_servers
                .insert(String::from("alpha"), with_endpoints(alpha.clone()));
        }
        harness.slots.set_deferred("alpha", true);
        gate.add_permits(1);
    };
    let (result, ()) = tokio::join!(harness.slots.ensure_discovered(&alpha), replace_twice);
    match expected_error {
        Some(error) => assert_eq!(result, Err(error)),
        None => assert_eq!(result.unwrap(), registered_db(&harness, "alpha").await),
    }
}
