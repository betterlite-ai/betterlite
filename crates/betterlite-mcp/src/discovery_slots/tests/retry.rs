#[tokio::test(start_paused = true)]
async fn retry_transport_failures_retry_at_the_configured_delays() {
    let alpha = server("alpha", "oauth2");
    let harness = build_harness(
        ScriptedFetcher::new(vec![(SERVER_URL, vec![Answer::Transport])]),
        registry_of(Vec::new(), vec![alpha.clone()]),
        DiscoverySettings::default(),
    );
    harness.slots.set_deferred("alpha", true);
    assert_eq!(
        harness.slots.ensure_discovered(&alpha).await,
        incomplete("alpha")
    );
    let times = harness.fetcher.call_times(SERVER_URL);
    assert_eq!(times.len(), 3);
    assert_eq!(times[1].duration_since(times[0]), Duration::from_millis(50));
    assert_eq!(
        times[2].duration_since(times[1]),
        Duration::from_millis(150)
    );
}

#[rstest]
#[case::no_retries(Vec::new(), 1)]
#[case::one_retry(vec![Duration::from_secs(1)], 2)]
#[tokio::test(start_paused = true)]
async fn retry_follows_shortened_delays(#[case] delays: Vec<Duration>, #[case] expected: usize) {
    let alpha = server("alpha", "oauth2");
    let harness = build_harness(
        ScriptedFetcher::new(vec![(SERVER_URL, vec![Answer::Transport])]),
        registry_of(Vec::new(), vec![alpha.clone()]),
        settings_with(false, delays),
    );
    harness.slots.set_deferred("alpha", true);
    assert_eq!(
        harness.slots.ensure_discovered(&alpha).await,
        incomplete("alpha")
    );
    let times = harness.fetcher.call_times(SERVER_URL);
    assert_eq!(times.len(), expected);
    if let [first, second] = times.as_slice() {
        assert_eq!(second.duration_since(*first), Duration::from_secs(1));
    }
}

#[rstest]
#[case::internal_error(StatusCode::INTERNAL_SERVER_ERROR, 3)]
#[case::unavailable(StatusCode::SERVICE_UNAVAILABLE, 3)]
#[case::not_found(StatusCode::NOT_FOUND, 1)]
#[case::unauthorized(StatusCode::UNAUTHORIZED, 1)]
#[case::ok_without_metadata(StatusCode::OK, 1)]
#[tokio::test(start_paused = true)]
async fn retry_only_server_errors_and_transport_failures(
    #[case] status: StatusCode,
    #[case] expected: usize,
) {
    let alpha = server("alpha", "oauth2");
    let harness = build_harness(
        ScriptedFetcher::new(vec![(SERVER_URL, vec![Answer::Document(status, "")])]),
        registry_of(Vec::new(), vec![alpha.clone()]),
        DiscoverySettings::default(),
    );
    harness.slots.set_deferred("alpha", true);
    assert_eq!(
        harness.slots.ensure_discovered(&alpha).await,
        incomplete("alpha")
    );
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), expected);
}

#[tokio::test(start_paused = true)]
async fn retry_then_success_resolves_the_server() {
    let alpha = server("alpha", "oauth2");
    let harness = build_harness(
        ScriptedFetcher::discoverable(vec![Answer::Transport, EMPTY_DOCUMENT]),
        registry_of(Vec::new(), vec![alpha.clone()]),
        DiscoverySettings::default(),
    );
    harness.slots.set_deferred("alpha", true);
    let resolved = harness.slots.ensure_discovered(&alpha).await.unwrap();
    assert_eq!(resolved, discovered(alpha));
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 2);
}

#[tokio::test(start_paused = true)]
async fn retry_run_times_out_after_the_metadata_timeout() {
    let alpha = server("alpha", "oauth2");
    let harness = build_harness(
        ScriptedFetcher::new(vec![(SERVER_URL, vec![Answer::Hang])]),
        registry_of(Vec::new(), vec![alpha.clone()]),
        DiscoverySettings::default(),
    );
    harness.slots.set_deferred("alpha", true);
    let started = Instant::now();
    let timed_out = Err(DiscoveryUnavailable::Failed {
        reason: FailureReason::TimedOut,
        server_ref: String::from("alpha"),
    });
    assert_eq!(harness.slots.ensure_discovered(&alpha).await, timed_out);
    assert_eq!(
        Instant::now().duration_since(started),
        Duration::from_secs(10)
    );
    assert_eq!(harness.slots.ensure_discovered(&alpha).await, timed_out);
    assert_eq!(harness.fetcher.calls_to(SERVER_URL), 1);
}

#[rstest]
#[case::transport(DiscoveryStatus::Transport(String::from("reset")), true)]
#[case::internal_error(DiscoveryStatus::HttpError(StatusCode::INTERNAL_SERVER_ERROR), true)]
#[case::bad_gateway(DiscoveryStatus::HttpError(StatusCode::BAD_GATEWAY), true)]
#[case::last_client_error(DiscoveryStatus::HttpError(StatusCode::from_u16(499).unwrap()), false)]
#[case::unauthorized(DiscoveryStatus::HttpError(StatusCode::UNAUTHORIZED), false)]
#[case::not_found(DiscoveryStatus::NotFound, false)]
#[case::invalid(DiscoveryStatus::Invalid, false)]
#[case::issuer_mismatch(DiscoveryStatus::IssuerMismatch, false)]
#[case::cross_origin(DiscoveryStatus::CrossOrigin, false)]
fn retry_classifies_discovery_statuses(#[case] status: DiscoveryStatus, #[case] expected: bool) {
    assert_eq!(is_retryable(&status), expected);
}

#[rstest]
#[case::oauth2("oauth2", None, None, (true, true, true))]
#[case::client_credentials("oauth2", Some(OAuth2Flow::ClientCredentials), None, (true, false, true))]
#[case::authorization_code("oauth2", Some(OAuth2Flow::AuthorizationCode), None, (true, true, true))]
#[case::passthrough("true_passthrough", None, None, (true, true, true))]
#[case::token_exchange("oauth2_token_exchange", None, None, (false, false, true))]
#[case::token_exchange_with_endpoint(
    "oauth2_token_exchange",
    None,
    Some("https://idp.test/exchange"),
    (false, false, false)
)]
#[case::api_key("api_key", None, None, (false, false, false))]
fn retry_discovery_needs_follow_the_auth_type(
    #[case] auth_type: &str,
    #[case] flow: Option<OAuth2Flow>,
    #[case] exchange_endpoint: Option<&str>,
    #[case] expected: (bool, bool, bool),
) {
    let mut alpha = server("alpha", auth_type);
    alpha.fields.oauth2_flow = flow;
    alpha.fields.token_exchange_endpoint = exchange_endpoint.map(str::to_owned);
    let needs = discovery_needs(&alpha);
    assert_eq!(
        (
            needs.is_discovery_auth_type,
            needs.authorization.needed,
            needs.token.needed
        ),
        expected
    );
}

#[rstest]
#[case::discovery_auth_type("oauth2", None)]
#[case::token_exchange("oauth2_token_exchange", Some("https://idp.test/token"))]
#[tokio::test(start_paused = true)]
async fn retry_discovered_tokens_need_a_corroborating_authorization_server(
    #[case] auth_type: &str,
    #[case] expected_token_url: Option<&str>,
) {
    let mut alpha = server("alpha", auth_type);
    alpha.fields.authorization_url = Some(String::from("https://other.test/authorize"));
    let fetcher = Arc::new(ScriptedFetcher::discoverable(vec![EMPTY_DOCUMENT]));
    let attempt = discover_server(&fetcher, FetchSettings::default(), &alpha).await;
    let metadata = attempt.metadata.unwrap();
    assert_eq!(metadata.token_url.as_deref(), expected_token_url);
    assert_eq!(
        metadata.authorization_url.as_deref(),
        Some("https://idp.test/authorize")
    );
    assert!(!attempt.retryable);
    assert_eq!(fetcher.called_urls()[0], SERVER_URL);
}

#[tokio::test(start_paused = true)]
async fn retry_anchored_issuer_metadata_is_returned_unfiltered() {
    let mut alpha = server("alpha", "oauth2");
    alpha.fields.issuer = Some(String::from("https://idp.test"));
    alpha.fields.issuer_is_anchored = true;
    alpha.fields.authorization_url = Some(String::from("https://other.test/authorize"));
    let fetcher = Arc::new(ScriptedFetcher::discoverable(vec![EMPTY_DOCUMENT]));
    let attempt = discover_server(&fetcher, FetchSettings::default(), &alpha).await;
    let metadata = attempt.metadata.unwrap();
    assert_eq!(
        metadata.token_url.as_deref(),
        Some("https://idp.test/token")
    );
    assert_eq!(fetcher.called_urls()[0], AUTHORIZATION_SERVER_URL);
}

#[tokio::test(start_paused = true)]
async fn retry_failed_discovery_reports_retryable_statuses() {
    let alpha = server("alpha", "oauth2");
    let fetcher = Arc::new(ScriptedFetcher::new(vec![(
        SERVER_URL,
        vec![Answer::Transport],
    )]));
    let attempt = discover_server(&fetcher, FetchSettings::default(), &alpha).await;
    assert_eq!(attempt.metadata, None);
    assert!(attempt.retryable);
}
