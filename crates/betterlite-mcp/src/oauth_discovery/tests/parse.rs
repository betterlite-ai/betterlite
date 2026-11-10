#[rstest]
#[case::quoted(
    "Bearer resource_metadata=\"https://mcp.test/prm\"",
    Some("https://mcp.test/prm")
)]
#[case::bare(
    "Bearer resource_metadata=https://mcp.test/prm",
    Some("https://mcp.test/prm")
)]
#[case::padded(
    "Bearer resource_metadata= \"https://mcp.test/prm\" ",
    Some("https://mcp.test/prm")
)]
#[case::parameter_case(
    "Bearer Resource_Metadata=\"https://mcp.test/prm\"",
    Some("https://mcp.test/prm")
)]
#[case::lowercase_scheme(
    "bearer resource_metadata=\"https://mcp.test/prm\"",
    Some("https://mcp.test/prm")
)]
#[case::after_other_parameters(
    "Bearer error=\"invalid_token\", resource_metadata=\"https://mcp.test/prm\"",
    Some("https://mcp.test/prm")
)]
#[case::space_inside_a_quoted_value(
    "Bearer error_description=\"The token expired\", resource_metadata=\"https://mcp.test/prm\"",
    Some("https://mcp.test/prm")
)]
#[case::comma_inside_a_quoted_value(
    "Bearer error=\"a,b\", resource_metadata=\"https://mcp.test/prm\"",
    Some("https://mcp.test/prm")
)]
#[case::spaces_around_equals(
    "Bearer error=\"x\", resource_metadata = \"https://mcp.test/prm\"",
    Some("https://mcp.test/prm")
)]
#[case::after_another_challenge(
    "DPoP algs=\"ES256\", Bearer resource_metadata=\"https://mcp.test/prm\"",
    Some("https://mcp.test/prm")
)]
#[case::between_challenges(
    "Basic realm=\"mcp\", Bearer resource_metadata=\"https://mcp.test/prm\", DPoP algs=\"ES256\"",
    Some("https://mcp.test/prm")
)]
#[case::relative_hint("Bearer resource_metadata=\"/prm\"", Some("/prm"))]
#[case::other_challenge("Basic resource_metadata=\"https://mcp.test/prm\"", None)]
#[case::no_scheme("resource_metadata=\"https://mcp.test/prm\"", None)]
#[case::no_parameter("Bearer realm=\"mcp\"", None)]
#[case::empty("", None)]
fn parse_www_authenticate_reads_a_bearer_resource_metadata(
    #[case] header: &str,
    #[case] expected: Option<&str>,
) {
    let parsed = parse_www_authenticate(header.as_bytes());
    assert_eq!(parsed.resource_metadata_url.as_deref(), expected);
}

#[test]
fn parse_www_authenticate_stops_a_value_at_a_comma() {
    let parsed = parse_www_authenticate(b"Bearer resource_metadata=\"https://mcp.test/a,b\"");
    assert_eq!(
        parsed.resource_metadata_url.as_deref(),
        Some("https://mcp.test/a")
    );
}

#[test]
fn parse_www_authenticate_keeps_the_last_occurrence() {
    let parsed = parse_www_authenticate(
        b"Bearer resource_metadata=\"https://mcp.test/first\", \
          resource_metadata=\"https://mcp.test/second\"",
    );
    assert_eq!(
        parsed.resource_metadata_url.as_deref(),
        Some("https://mcp.test/second")
    );
}

#[rstest]
#[case::quoted("Bearer scope=\"read write\"", Some(vec!["read", "write"]))]
#[case::bare("Bearer scope=read", Some(vec!["read"]))]
#[case::blank("Bearer scope=\"   \"", None)]
#[case::absent("Bearer resource_metadata=\"https://mcp.test/prm\"", None)]
fn parse_www_authenticate_reads_the_challenge_scope(
    #[case] header: &str,
    #[case] expected: Option<Vec<&str>>,
) {
    let parsed = parse_www_authenticate(header.as_bytes());
    assert_eq!(parsed.scopes, expected.map(|values| strings(&values)));
}

#[rstest]
#[case::invalid_utf8(b"Bearer \xff\xfe".as_slice())]
#[case::only_scheme(b"Bearer".as_slice())]
#[case::scheme_and_space(b"Bearer ".as_slice())]
#[case::empty_value(b"Bearer resource_metadata=".as_slice())]
#[case::unterminated_quote(b"Bearer resource_metadata=\"https://mcp.test".as_slice())]
#[case::equals_only(b"Bearer =".as_slice())]
#[case::nul_bytes(b"Bearer \x00=\x00".as_slice())]
fn parse_www_authenticate_never_panics_on_arbitrary_bytes(#[case] header: &[u8]) {
    drop(parse_www_authenticate(header));
}

#[rstest]
#[case::default_port(
    "https://mcp.test/.well-known/oauth-protected-resource",
    "https://mcp.test/mcp",
    true
)]
#[case::explicit_default_port("https://mcp.test:443/prm", "https://mcp.test/mcp", true)]
#[case::http_default_port("http://mcp.test/prm", "http://mcp.test/mcp", true)]
#[case::same_explicit_port("https://mcp.test:8443/prm", "https://mcp.test:8443/mcp", true)]
#[case::other_port("https://mcp.test:8443/prm", "https://mcp.test/mcp", false)]
#[case::other_host("https://idp.test/prm", "https://mcp.test/mcp", false)]
#[case::other_scheme("http://mcp.test/prm", "https://mcp.test/mcp", false)]
#[case::other_scheme_same_port("https://mcp.test:8443/prm", "http://mcp.test:8443/mcp", false)]
#[case::case_insensitive_host("https://MCP.test/prm", "https://mcp.test/mcp", true)]
#[case::userinfo_is_ignored("https://user:pw@mcp.test/prm", "https://mcp.test/mcp", true)]
#[case::relative_hint("/prm", "https://mcp.test/mcp", false)]
#[case::non_http_hint("ftp://mcp.test/prm", "ftp://mcp.test/mcp", false)]
#[case::unparsable_hint("https://[::1/prm", "https://mcp.test/mcp", false)]
#[case::out_of_range_port("https://mcp.test:70000/prm", "https://mcp.test/mcp", false)]
fn parse_same_authority_compares_scheme_host_and_port(
    #[case] url: &str,
    #[case] server_url: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_same_authority_metadata_url(url, server_url), expected);
}

#[tokio::test]
async fn parse_cross_authority_hint_is_not_followed_and_warns() {
    let (log, _guard) = capture();
    let hint = "https://evil.test/.well-known/oauth-protected-resource";
    let mut answers = vec![(
        "https://mcp.test/mcp",
        challenge_document(&format!("Bearer resource_metadata=\"{hint}\"")),
    )];
    answers.push((
        "https://evil.test/.well-known/oauth-protected-resource",
        document("{}"),
    ));
    let fetcher = FakeFetcher::new(answers);
    let report =
        discover_recording_attempts(&fetcher, settings(), request("https://mcp.test/mcp")).await;
    assert!(!fetcher.urls().iter().any(|url| url.contains("evil.test")));
    assert!(matches!(
        &report.outcome,
        DiscoveryOutcome::Failed(statuses) if statuses.contains(&DiscoveryStatus::CrossOrigin)
    ));
    let logged = log.text();
    assert!(logged.contains("https://evil.test"));
    assert!(logged.contains("https://mcp.test"));
}
