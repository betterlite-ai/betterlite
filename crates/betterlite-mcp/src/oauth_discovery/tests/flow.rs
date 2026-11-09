fn discovered_metadata(issuer: &str) -> MCPOAuthMetadata {
    MCPOAuthMetadata {
        scopes: Some(strings(&["read"])),
        authorization_url: Some(format!("{issuer}/authorize")),
        token_url: Some(format!("{issuer}/token")),
        registration_url: Some(format!("{issuer}/register")),
        discovered_issuer: Some(issuer.to_owned()),
        from_origin_fallback: false,
    }
}

fn metadata_with_scopes(scopes: Vec<String>) -> MCPOAuthMetadata {
    MCPOAuthMetadata {
        scopes: Some(scopes),
        authorization_url: None,
        token_url: None,
        registration_url: None,
        discovered_issuer: None,
        from_origin_fallback: false,
    }
}

#[test]
fn flow_default_metadata_timeout_is_ten_seconds() {
    assert_eq!(settings().metadata_timeout, Duration::from_secs(10));
}

#[rstest]
#[case::absent(None, Duration::from_secs(10))]
#[case::empty(Some(""), Duration::from_secs(10))]
#[case::blank(Some("   "), Duration::from_secs(10))]
#[case::fractional(Some("2.5"), Duration::from_millis(2500))]
#[case::padded(Some(" 5 "), Duration::from_secs(5))]
#[case::zero(Some("0"), Duration::from_secs(10))]
#[case::negative(Some("-3"), Duration::from_secs(10))]
#[case::not_a_number(Some("soon"), Duration::from_secs(10))]
#[case::infinite(Some("inf"), Duration::from_secs(10))]
fn flow_metadata_timeout_reads_the_environment(
    #[case] value: Option<&str>,
    #[case] expected: Duration,
) {
    let mut environment = BTreeMap::new();
    if let Some(text) = value {
        let previous = environment.insert(String::from(METADATA_TIMEOUT_VARIABLE), text.to_owned());
        assert_eq!(previous, None);
    }
    assert_eq!(
        DiscoverySettings::from_lookup(&environment).metadata_timeout,
        expected
    );
}

#[test]
fn flow_an_invalid_metadata_timeout_warns() {
    let (log, _guard) = capture();
    let environment = BTreeMap::from([(
        String::from(METADATA_TIMEOUT_VARIABLE),
        String::from("soon"),
    )]);
    assert_eq!(
        DiscoverySettings::from_lookup(&environment).metadata_timeout,
        Duration::from_secs(10)
    );
    assert!(log.text().contains(METADATA_TIMEOUT_VARIABLE));
}

#[tokio::test]
async fn flow_discovery_uses_the_configured_metadata_timeout() {
    let fetcher = FakeFetcher::default();
    let timeout = Duration::from_millis(250);
    discover_metadata(
        &fetcher,
        settings_with_timeout(timeout),
        request("https://mcp.test/mcp"),
    )
    .await;
    assert!(fetcher.calls().iter().all(|call| call.timeout == timeout));
}

#[tokio::test]
async fn flow_well_known_candidates_try_the_path_aware_url_first() {
    let fetcher = FakeFetcher::new(vec![
        ("https://mcp.test/mcp", document("{}")),
        (
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            document(&resource_document(&["https://idp.test"], None)),
        ),
        (
            "https://idp.test/.well-known/oauth-authorization-server",
            document(&authorization_server_document("https://idp.test")),
        ),
    ]);
    let outcome = discover_metadata(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert_eq!(
        fetcher.urls(),
        strings(&[
            "https://mcp.test/mcp",
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            "https://idp.test/.well-known/oauth-authorization-server",
        ])
    );
    assert_eq!(
        outcome,
        DiscoveryOutcome::Found(discovered_metadata("https://idp.test"))
    );
}

#[tokio::test]
async fn flow_well_known_falls_back_to_the_root_candidate() {
    let fetcher = FakeFetcher::new(vec![
        ("https://mcp.test/mcp", document("{}")),
        (
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            status_document(StatusCode::NOT_FOUND),
        ),
        (
            "https://mcp.test/.well-known/oauth-protected-resource",
            document(&resource_document(&["https://idp.test"], None)),
        ),
        (
            "https://idp.test/.well-known/oauth-authorization-server",
            document(&authorization_server_document("https://idp.test")),
        ),
    ]);
    let outcome = discover_metadata(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert_eq!(
        fetcher.urls(),
        strings(&[
            "https://mcp.test/mcp",
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            "https://mcp.test/.well-known/oauth-protected-resource",
            "https://idp.test/.well-known/oauth-authorization-server",
        ])
    );
    assert_eq!(
        outcome,
        DiscoveryOutcome::Found(discovered_metadata("https://idp.test"))
    );
}

#[tokio::test]
async fn flow_well_known_continues_after_an_invalid_candidate() {
    let fetcher = FakeFetcher::new(vec![
        ("https://mcp.test/mcp", document("{}")),
        (
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            document("[]"),
        ),
        (
            "https://mcp.test/.well-known/oauth-protected-resource",
            document(&resource_document(&["https://idp.test"], None)),
        ),
        (
            "https://idp.test/.well-known/oauth-authorization-server",
            document(&authorization_server_document("https://idp.test")),
        ),
    ]);
    let outcome = discover_metadata(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert_eq!(
        outcome,
        DiscoveryOutcome::Found(discovered_metadata("https://idp.test"))
    );
}

#[tokio::test]
async fn flow_well_known_lookup_reports_every_absence() {
    let fetcher = FakeFetcher::new(vec![
        (
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            status_document(StatusCode::NOT_FOUND),
        ),
        (
            "https://mcp.test/.well-known/oauth-protected-resource",
            status_document(StatusCode::GONE),
        ),
    ]);
    let lookup = attempt_well_known_discovery(&fetcher, "https://mcp.test/mcp", settings()).await;
    assert!(lookup.authorization_servers.is_empty());
    assert_eq!(
        lookup.statuses,
        vec![DiscoveryStatus::NotFound, DiscoveryStatus::NotFound]
    );
}

#[tokio::test]
async fn flow_resource_scopes_override_the_authorization_server_scopes() {
    let fetcher = FakeFetcher::new(vec![
        ("https://mcp.test/mcp", document("{}")),
        (
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            document(&resource_document(
                &["https://idp.test"],
                Some(&["resource"]),
            )),
        ),
        (
            "https://idp.test/.well-known/oauth-authorization-server",
            document(&authorization_server_document("https://idp.test")),
        ),
    ]);
    let outcome = discover_metadata(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert_eq!(
        outcome,
        DiscoveryOutcome::Found(MCPOAuthMetadata {
            scopes: Some(strings(&["resource"])),
            ..discovered_metadata("https://idp.test")
        })
    );
}

#[tokio::test]
async fn flow_missing_authorization_metadata_still_returns_resource_scopes() {
    let fetcher = FakeFetcher::new(vec![
        ("https://mcp.test/mcp", document("{}")),
        (
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            document(&resource_document(
                &["https://idp.test"],
                Some(&["resource"]),
            )),
        ),
    ]);
    let outcome = discover_metadata(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert_eq!(
        outcome,
        DiscoveryOutcome::Found(metadata_with_scopes(strings(&["resource"])))
    );
}

#[tokio::test]
async fn flow_authorization_server_documents_are_tried_in_rfc_order() {
    let fetcher = FakeFetcher::new(vec![
        ("https://mcp.test/mcp", document("{}")),
        (
            "https://mcp.test/.well-known/oauth-protected-resource",
            document(&resource_document(&["https://idp.test/tenant"], None)),
        ),
        (
            "https://idp.test/tenant",
            document(&authorization_server_document("https://idp.test/tenant")),
        ),
    ]);
    let outcome = discover_metadata(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    let issuer_calls: Vec<String> = fetcher
        .urls()
        .into_iter()
        .filter(|url| url.starts_with("https://idp.test"))
        .collect();
    assert_eq!(
        issuer_calls,
        strings(&[
            "https://idp.test/.well-known/oauth-authorization-server/tenant",
            "https://idp.test/.well-known/openid-configuration/tenant",
            "https://idp.test/tenant/.well-known/openid-configuration",
            "https://idp.test/.well-known/oauth-authorization-server",
            "https://idp.test/.well-known/openid-configuration",
            "https://idp.test/tenant",
        ])
    );
    assert_eq!(
        outcome,
        DiscoveryOutcome::Found(discovered_metadata("https://idp.test/tenant"))
    );
}

#[tokio::test]
async fn flow_authorization_metadata_tries_each_issuer_in_order() {
    let fetcher = FakeFetcher::new(vec![
        (
            "https://first.test/.well-known/oauth-authorization-server",
            status_document(StatusCode::INTERNAL_SERVER_ERROR),
        ),
        (
            "https://second.test/.well-known/oauth-authorization-server",
            document(&authorization_server_document("https://second.test")),
        ),
    ]);
    let issuers = strings(&[
        "https://first.test",
        "https://second.test",
        "https://third.test",
    ]);
    let lookup = fetch_authorization_metadata(&fetcher, &issuers, settings()).await;
    assert_eq!(
        lookup.metadata,
        Some(discovered_metadata("https://second.test"))
    );
    assert_eq!(
        lookup.statuses.first(),
        Some(&DiscoveryStatus::HttpError(
            StatusCode::INTERNAL_SERVER_ERROR
        ))
    );
    assert!(!fetcher.urls().iter().any(|url| url.contains("third.test")));
}

#[tokio::test]
async fn flow_issuer_anchored_accepts_the_configured_issuer() {
    let fetcher = FakeFetcher::new(vec![(
        "https://idp.test/.well-known/oauth-authorization-server",
        document(&authorization_server_document("https://idp.test")),
    )]);
    let outcome =
        fetch_issuer_anchored_metadata(&fetcher, "https://idp.test", None, settings()).await;
    assert_eq!(
        outcome,
        DiscoveryOutcome::Found(MCPOAuthMetadata {
            scopes: None,
            ..discovered_metadata("https://idp.test")
        })
    );
}

#[tokio::test]
async fn flow_issuer_anchored_rejects_a_mismatched_issuer() {
    let (log, _guard) = capture();
    let fetcher = FakeFetcher::new(vec![
        (
            "https://idp.test/.well-known/oauth-authorization-server",
            document(&authorization_server_document("https://evil.test")),
        ),
        (
            "https://idp.test/.well-known/openid-configuration",
            document(&authorization_server_document("https://evil.test")),
        ),
        (
            "https://idp.test",
            document(&authorization_server_document("https://evil.test")),
        ),
    ]);
    let outcome =
        fetch_issuer_anchored_metadata(&fetcher, "https://idp.test", None, settings()).await;
    let DiscoveryOutcome::Failed(statuses) = outcome else {
        panic!("an issuer mismatch must leave the endpoints unresolved");
    };
    assert_eq!(statuses.len(), 3);
    assert!(
        statuses
            .iter()
            .all(|status| *status == DiscoveryStatus::IssuerMismatch)
    );
    assert!(log.text().contains("does not self-attest"));
}

#[tokio::test]
async fn flow_issuer_anchored_takes_scopes_from_the_resource_document() {
    let fetcher = FakeFetcher::new(vec![
        ("https://mcp.test/mcp", document("{}")),
        (
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            document(&resource_document(
                &["https://other.test"],
                Some(&["resource"]),
            )),
        ),
        (
            "https://idp.test/.well-known/oauth-authorization-server",
            document(&authorization_server_document("https://idp.test")),
        ),
    ]);
    let outcome = fetch_issuer_anchored_metadata(
        &fetcher,
        "https://idp.test",
        Some("https://mcp.test/mcp"),
        settings(),
    )
    .await;
    assert_eq!(
        outcome,
        DiscoveryOutcome::Found(MCPOAuthMetadata {
            scopes: Some(strings(&["resource"])),
            ..discovered_metadata("https://idp.test")
        })
    );
}

#[rstest]
#[case::global("login.microsoftonline.com")]
#[case::us_government("login.microsoftonline.us")]
#[case::china("login.chinacloudapi.cn")]
#[tokio::test]
async fn flow_azure_tenant_url_synthesizes_the_endpoints(#[case] host: &str) {
    let issuer = format!("https://{host}/tenant/v2.0");
    let fetcher = FakeFetcher::default();
    let lookup = fetch_single_issuer_metadata(&fetcher, &issuer, None, settings()).await;
    assert_eq!(
        lookup.metadata,
        Some(MCPOAuthMetadata {
            scopes: None,
            authorization_url: Some(format!("https://{host}/tenant/oauth2/v2.0/authorize")),
            token_url: Some(format!("https://{host}/tenant/oauth2/v2.0/token")),
            registration_url: None,
            discovered_issuer: None,
            from_origin_fallback: false,
        })
    );
}

#[rstest]
#[case::version_one("https://login.microsoftonline.com/tenant/v1.0")]
#[case::no_version("https://login.microsoftonline.com/tenant")]
#[case::extra_segment("https://login.microsoftonline.com/tenant/v2.0/extra")]
#[case::other_host("https://idp.test/tenant/v2.0")]
#[case::explicit_port("https://login.microsoftonline.com:443/tenant/v2.0")]
#[case::uppercase_host("https://LOGIN.MICROSOFTONLINE.COM/tenant/v2.0")]
#[tokio::test]
async fn flow_azure_fallback_needs_a_two_segment_v2_0_path(#[case] issuer: &str) {
    let fetcher = FakeFetcher::default();
    let lookup = fetch_single_issuer_metadata(&fetcher, issuer, None, settings()).await;
    assert!(lookup.metadata.is_none());
}

#[test]
fn flow_azure_hosts_cover_global_us_and_china() {
    assert_eq!(
        AZURE_ENTRA_HOSTS,
        [
            "login.microsoftonline.com",
            "login.microsoftonline.us",
            "login.chinacloudapi.cn"
        ]
    );
}

#[tokio::test]
async fn flow_origin_fallback_marks_the_metadata() {
    let fetcher = FakeFetcher::new(vec![
        (
            "https://mcp.test/mcp",
            challenge_document("Bearer realm=\"mcp\""),
        ),
        (
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            status_document(StatusCode::NOT_FOUND),
        ),
        (
            "https://mcp.test/.well-known/oauth-protected-resource",
            status_document(StatusCode::NOT_FOUND),
        ),
        (
            "https://mcp.test/.well-known/oauth-authorization-server",
            document(&authorization_server_document("https://mcp.test")),
        ),
    ]);
    let outcome = discover_metadata(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert_eq!(
        outcome,
        DiscoveryOutcome::Found(MCPOAuthMetadata {
            from_origin_fallback: true,
            ..discovered_metadata("https://mcp.test")
        })
    );
}

#[tokio::test]
async fn flow_status_error_without_origin_fallback_fails() {
    let fetcher = FakeFetcher::new(vec![
        (
            "https://mcp.test/mcp",
            challenge_document("Bearer realm=\"mcp\""),
        ),
        (
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            status_document(StatusCode::NOT_FOUND),
        ),
        (
            "https://mcp.test/.well-known/oauth-protected-resource",
            status_document(StatusCode::NOT_FOUND),
        ),
    ]);
    let outcome =
        discover_metadata(&fetcher, settings(), quiet_request("https://mcp.test/mcp")).await;
    assert_eq!(
        outcome,
        DiscoveryOutcome::Failed(vec![
            DiscoveryStatus::HttpError(StatusCode::UNAUTHORIZED),
            DiscoveryStatus::NotFound,
            DiscoveryStatus::NotFound,
        ])
    );
    assert!(!fetcher.urls().iter().any(|url| url == "https://mcp.test"));
}

#[tokio::test]
async fn flow_401_hint_uses_the_advertised_resource_document() {
    let fetcher = FakeFetcher::new(vec![
        (
            "https://mcp.test/mcp",
            challenge_document("Bearer resource_metadata=\"https://mcp.test/prm\", scope=\"read\""),
        ),
        (
            "https://mcp.test/prm",
            document(&resource_document(
                &["https://idp.test"],
                Some(&["resource"]),
            )),
        ),
        (
            "https://idp.test/.well-known/oauth-authorization-server",
            document(&authorization_server_document("https://idp.test")),
        ),
    ]);
    let outcome = discover_metadata(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert_eq!(
        fetcher.urls(),
        strings(&[
            "https://mcp.test/mcp",
            "https://mcp.test/prm",
            "https://idp.test/.well-known/oauth-authorization-server",
        ])
    );
    assert_eq!(
        outcome,
        DiscoveryOutcome::Found(MCPOAuthMetadata {
            scopes: Some(strings(&["read"])),
            ..discovered_metadata("https://idp.test")
        })
    );
}

#[tokio::test]
async fn flow_report_records_each_401_attempt() {
    let fetcher = FakeFetcher::new(vec![
        (
            "https://mcp.test/mcp",
            challenge_document("Bearer resource_metadata=\"https://evil.test/prm\""),
        ),
        (
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            status_document(StatusCode::NOT_FOUND),
        ),
        (
            "https://mcp.test/.well-known/oauth-protected-resource",
            status_document(StatusCode::NOT_FOUND),
        ),
    ]);
    let report =
        discover_recording_attempts(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert_eq!(report.attempts.len(), 3);
    assert!(report.attempts[0].contains("HTTP 401"));
    assert!(report.attempts[1].contains("challenge-advertised resource metadata yielded no"),);
    assert!(report.attempts[2].contains("origin fallback"));
}

#[tokio::test]
async fn flow_warns_when_a_200_server_has_no_discoverable_metadata() {
    let (log, _guard) = capture();
    let fetcher = FakeFetcher::new(vec![
        ("https://mcp.test/mcp", document("{}")),
        (
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            status_document(StatusCode::NOT_FOUND),
        ),
        (
            "https://mcp.test/.well-known/oauth-protected-resource",
            document(&resource_document(&["https://idp.test"], None)),
        ),
    ]);
    let outcome = discover_metadata(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert!(matches!(outcome, DiscoveryOutcome::Failed(_)));
    assert!(
        log.text()
            .contains("received 200 OK without an RFC 9728 challenge")
    );
}

#[tokio::test]
async fn flow_discover_metadata_warns_when_no_metadata_is_found() {
    let (log, _guard) = capture();
    let fetcher = FakeFetcher::new(vec![(
        "https://mcp.test/mcp",
        status_document(StatusCode::NOT_FOUND),
    )]);
    let outcome = discover_metadata(
        &fetcher,
        settings(),
        warning_request("https://mcp.test/mcp"),
    )
    .await;
    assert!(matches!(outcome, DiscoveryOutcome::Failed(_)));
    assert!(
        log.text()
            .contains("found no authorization server metadata")
    );
    assert!(log.text().contains("well-known protected-resource lookup"));
}

#[tokio::test]
async fn flow_discover_metadata_stays_quiet_without_the_flag() {
    let (log, _guard) = capture();
    let fetcher = FakeFetcher::new(vec![(
        "https://mcp.test/mcp",
        status_document(StatusCode::NOT_FOUND),
    )]);
    let outcome = discover_metadata(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert!(matches!(outcome, DiscoveryOutcome::Failed(_)));
    assert!(
        !log.text()
            .contains("found no authorization server metadata")
    );
}

#[tokio::test]
async fn flow_a_transport_failure_on_the_server_url_is_recorded() {
    let fetcher = FakeFetcher::new(vec![(
        "https://mcp.test/mcp",
        transport_error("connect to https://mcp.test/mcp?token=abc failed"),
    )]);
    let report =
        discover_recording_attempts(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert_eq!(
        report.outcome,
        DiscoveryOutcome::Failed(vec![DiscoveryStatus::Transport(String::from(
            "connect to <url> failed"
        ))])
    );
    assert_eq!(report.attempts.len(), 1);
    assert!(report.attempts[0].contains("transport error: connect to <url> failed"));
}

#[rstest]
#[case::empty("")]
#[case::no_scheme("//mcp.test/mcp")]
#[case::no_authority("mailto:issuer@mcp.test")]
#[tokio::test]
async fn flow_well_known_without_a_usable_url_tries_nothing(#[case] server_url: &'static str) {
    let fetcher = FakeFetcher::default();
    let lookup = attempt_well_known_discovery(&fetcher, server_url, settings()).await;
    assert!(lookup.statuses.is_empty());
    assert!(fetcher.urls().is_empty());
}

#[rstest]
#[case::no_scheme("//idp.test")]
#[case::no_authority("mailto:issuer@idp.test")]
#[tokio::test]
async fn flow_an_issuer_without_an_authority_tries_nothing(#[case] issuer: &'static str) {
    let fetcher = FakeFetcher::default();
    let lookup = fetch_single_issuer_metadata(&fetcher, issuer, None, settings()).await;
    assert!(lookup.metadata.is_none());
    assert!(fetcher.urls().is_empty());
}

#[rstest]
#[case::scopes_only("{\"scopes_supported\":[\"read\"]}")]
#[case::authorization_only("{\"authorization_endpoint\":\"https://idp.test/authorize\"}")]
#[case::token_only("{\"token_endpoint\":\"https://idp.test/token\"}")]
#[case::registration_only("{\"registration_endpoint\":\"https://idp.test/register\"}")]
#[tokio::test]
async fn flow_a_document_with_one_field_is_enough(#[case] body: &str) {
    let fetcher = FakeFetcher::new(vec![(
        "https://idp.test/.well-known/oauth-authorization-server",
        document(body),
    )]);
    let lookup = fetch_single_issuer_metadata(&fetcher, "https://idp.test", None, settings()).await;
    assert!(lookup.metadata.is_some());
}

#[tokio::test]
async fn flow_a_document_without_fields_is_not_metadata() {
    let fetcher = FakeFetcher::new(vec![(
        "https://idp.test/.well-known/oauth-authorization-server",
        document("{}"),
    )]);
    let lookup = fetch_single_issuer_metadata(&fetcher, "https://idp.test", None, settings()).await;
    assert!(lookup.metadata.is_none());
    assert_eq!(fetcher.urls().len(), 3);
}

#[tokio::test]
async fn flow_found_metadata_without_resource_scopes_does_not_warn() {
    let (log, _guard) = capture();
    let fetcher = FakeFetcher::new(vec![
        ("https://mcp.test/mcp", document("{}")),
        (
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            document(&resource_document(&["https://idp.test"], None)),
        ),
        (
            "https://idp.test/.well-known/oauth-authorization-server",
            document(&authorization_server_document("https://idp.test")),
        ),
    ]);
    let outcome = discover_metadata(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert!(matches!(outcome, DiscoveryOutcome::Found(_)));
    assert!(
        !log.text()
            .contains("received 200 OK without an RFC 9728 challenge")
    );
}

#[tokio::test]
async fn flow_a_200_server_without_authorization_servers_does_not_warn() {
    let (log, _guard) = capture();
    let fetcher = FakeFetcher::new(vec![
        ("https://mcp.test/mcp", document("{}")),
        (
            "https://mcp.test/.well-known/oauth-protected-resource/mcp",
            status_document(StatusCode::NOT_FOUND),
        ),
        (
            "https://mcp.test/.well-known/oauth-protected-resource",
            status_document(StatusCode::NOT_FOUND),
        ),
    ]);
    let report =
        discover_recording_attempts(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert_eq!(
        report.attempts,
        strings(&[
            "GET https://mcp.test: HTTP 200 (no RFC 9728 challenge)",
            "well-known protected-resource lookup found no authorization servers",
        ])
    );
    assert!(
        !log.text()
            .contains("received 200 OK without an RFC 9728 challenge")
    );
}

#[rstest]
#[case::no_scheme("//mcp.test/mcp")]
#[case::no_authority("mailto:issuer@mcp.test")]
#[tokio::test]
async fn flow_origin_fallback_needs_a_server_authority(#[case] server_url: &'static str) {
    let fetcher = FakeFetcher::new(vec![(
        server_url,
        challenge_document("Bearer realm=\"mcp\""),
    )]);
    let report = discover_recording_attempts(&fetcher, settings(), request(server_url)).await;
    assert_eq!(report.attempts.len(), 2);
    assert!(matches!(report.outcome, DiscoveryOutcome::Failed(_)));
}

#[tokio::test]
async fn flow_a_bearer_challenge_in_a_later_header_field_is_read() {
    let fetcher = FakeFetcher::new(vec![
        (
            "https://mcp.test/mcp",
            challenge_fields_document(vec![
                ("www-authenticate", "DPoP algs=\"ES256\""),
                (
                    "www-authenticate",
                    "Bearer resource_metadata=\"https://mcp.test/prm\"",
                ),
            ]),
        ),
        (
            "https://mcp.test/prm",
            document(&resource_document(&["https://idp.test"], None)),
        ),
        (
            "https://idp.test/.well-known/oauth-authorization-server",
            document(&authorization_server_document("https://idp.test")),
        ),
    ]);
    let outcome = discover_metadata(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert_eq!(
        fetcher.urls(),
        strings(&[
            "https://mcp.test/mcp",
            "https://mcp.test/prm",
            "https://idp.test/.well-known/oauth-authorization-server",
        ])
    );
    assert_eq!(
        outcome,
        DiscoveryOutcome::Found(discovered_metadata("https://idp.test"))
    );
}

#[rstest]
fn replays_the_oauth_metadata_corpus(#[files("fuzz/corpus/oauth_metadata/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(parse_www_authenticate(&bytes));
    drop(decode_metadata_document(&bytes));
}
