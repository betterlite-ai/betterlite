const METADATA_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/identity";

fn answer(body: &str) -> Reply {
    Reply::Answer {
        status: 200,
        body: body.as_bytes().to_vec(),
    }
}

fn actions_environment() -> BTreeMap<String, String> {
    environment(&[
        (
            "ACTIONS_ID_TOKEN_REQUEST_URL",
            "https://actions.example/token",
        ),
        ("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "request-token"),
    ])
}

#[test]
fn oidc_google_requests_the_metadata_identity_endpoint() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, http) = scripted_lookup(vec![answer("google-token")], &clock);
    let value = lookup
        .get_secret(
            &BTreeMap::new(),
            "oidc/google/https://example.com/audience",
            None,
        )
        .expect("the Google provider should succeed");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("google-token"));
    let requests = http.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.url, METADATA_URL);
    assert_eq!(
        request.query,
        vec![(
            "audience".to_owned(),
            "https://example.com/audience".to_owned()
        )]
    );
    assert_eq!(
        request.headers,
        vec![("Metadata-Flavor".to_owned(), "Google".to_owned())]
    );
    assert_eq!(request.timeouts, OidcHttpTimeouts::DEFAULT);
}

#[test]
fn oidc_audience_keeps_an_os_environ_segment_when_the_name_does_not_start_with_it() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, http) = scripted_lookup(vec![answer("google-token")], &clock);
    lookup
        .get_secret(
            &BTreeMap::new(),
            "oidc/google/https://host/os.environ/audience",
            None,
        )
        .expect("the Google provider should succeed");
    assert_eq!(
        http.requests()[0].query,
        vec![(
            "audience".to_owned(),
            "https://host/os.environ/audience".to_owned()
        )]
    );
}

#[test]
fn oidc_google_failure_status_is_an_error() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, _http) = scripted_lookup(
        vec![Reply::Answer {
            status: 500,
            body: Vec::new(),
        }],
        &clock,
    );
    let error = lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect_err("a non-200 status is an error");
    assert_eq!(error.to_string(), "Google OIDC provider failed");
}

#[test]
fn oidc_google_transport_failure_is_an_error() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, _http) = scripted_lookup(vec![Reply::Failure], &clock);
    let error = lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect_err("a transport failure is an error");
    assert_eq!(error.to_string(), "OIDC request failed: scripted failure");
}

#[test]
fn oidc_google_invalid_utf8_is_an_error() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, _http) = scripted_lookup(
        vec![Reply::Answer {
            status: 200,
            body: vec![0xde, 0xad, 0xbe, 0xef],
        }],
        &clock,
    );
    let error = lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect_err("invalid UTF-8 is an error");
    assert_eq!(error.to_string(), "Google OIDC token response is not UTF-8");
    assert!(!format!("{error:?}").contains("222"));
}

#[test]
fn oidc_google_caches_a_token_for_the_default_ttl() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, http) =
        scripted_lookup(vec![answer("google-token"), answer("second-token")], &clock);
    let first = lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect("the first read should succeed");
    assert_eq!(text_value(first.as_ref()).as_deref(), Some("google-token"));
    clock.advance(3539);
    let cached = lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect("the cached read should succeed");
    assert_eq!(text_value(cached.as_ref()).as_deref(), Some("google-token"));
    assert_eq!(http.requests().len(), 1);
    clock.advance(2);
    let refreshed = lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect("the expired read should succeed");
    assert_eq!(
        text_value(refreshed.as_ref()).as_deref(),
        Some("second-token")
    );
    assert_eq!(http.requests().len(), 2);
}

#[test]
fn oidc_google_cache_entry_expires_exactly_at_its_ttl() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, http) =
        scripted_lookup(vec![answer("google-token"), answer("second-token")], &clock);
    let first = lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect("the first read should succeed");
    assert_eq!(text_value(first.as_ref()).as_deref(), Some("google-token"));
    clock.advance(3540);
    let refreshed = lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect("an entry expiring exactly now is a miss");
    assert_eq!(
        text_value(refreshed.as_ref()).as_deref(),
        Some("second-token")
    );
    assert_eq!(http.requests().len(), 2);
}

#[test]
fn oidc_google_token_with_a_zero_ttl_is_not_cached_and_warns() {
    let (logs, _guard) = capture_logs();
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let token = jwt(1_700_000_060);
    let (lookup, http) = scripted_lookup(vec![answer(&token), answer(&token)], &clock);
    lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect("a token expiring in exactly 60s is still returned");
    lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect("a token expiring in exactly 60s is still returned");
    assert_eq!(http.requests().len(), 2);
    assert!(logs.text().contains(
        "Google OIDC token for oidc/google/audience is already expired or expires within 60s; not caching it"
    ));
}

#[test]
fn oidc_google_caps_the_cache_at_the_token_expiry() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let token = jwt(1_700_000_600);
    let (lookup, http) = scripted_lookup(vec![answer(&token), answer("second-token")], &clock);
    lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect("the first read should succeed");
    clock.advance(539);
    lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect("the cached read should succeed");
    assert_eq!(http.requests().len(), 1);
    clock.advance(2);
    lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect("the expired read should succeed");
    assert_eq!(http.requests().len(), 2);
}

#[test]
fn oidc_google_expired_token_is_not_cached_and_warns() {
    let (logs, _guard) = capture_logs();
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let token = jwt(1_700_000_030);
    let (lookup, http) = scripted_lookup(vec![answer(&token), answer(&token)], &clock);
    lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect("an expired token is still returned");
    lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect("an expired token is still returned");
    assert_eq!(http.requests().len(), 2);
    assert!(logs.text().contains(
        "Google OIDC token for oidc/google/audience is already expired or expires within 60s; not caching it"
    ));
}

#[test]
fn oidc_google_cache_key_is_the_resolved_name() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, http) = scripted_lookup(vec![answer("google-token")], &clock);
    lookup
        .get_secret(&BTreeMap::new(), "os.environ/oidc/google/audience", None)
        .expect("the prefixed read should succeed");
    let cached = lookup
        .get_secret(&BTreeMap::new(), "oidc/google/audience", None)
        .expect("the unprefixed read should hit the same cache entry");
    assert_eq!(text_value(cached.as_ref()).as_deref(), Some("google-token"));
    assert_eq!(http.requests().len(), 1);
}

#[rstest]
#[case::circleci("oidc/circleci/audience", "CIRCLE_OIDC_TOKEN")]
#[case::circleci_v2("oidc/circleci_v2/audience", "CIRCLE_OIDC_TOKEN_V2")]
fn oidc_circleci_providers_read_their_environment_variable(
    #[case] secret_name: &str,
    #[case] variable: &str,
) {
    let environment = environment(&[(variable, "circle-token")]);
    let value = base_lookup()
        .get_secret(&environment, secret_name, None)
        .expect("the CircleCI provider should succeed");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("circle-token"));
}

#[rstest]
#[case::circleci("oidc/circleci/audience", "CIRCLE_OIDC_TOKEN not found in environment")]
#[case::circleci_v2(
    "oidc/circleci_v2/audience",
    "CIRCLE_OIDC_TOKEN_V2 not found in environment"
)]
fn oidc_circleci_missing_token_is_an_error(#[case] secret_name: &str, #[case] message: &str) {
    let error = base_lookup()
        .get_secret(&BTreeMap::new(), secret_name, None)
        .expect_err("a missing CircleCI token is an error");
    assert_eq!(error.to_string(), message);
}

#[rstest]
#[case::no_url(&[("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "request-token")])]
#[case::no_token(&[("ACTIONS_ID_TOKEN_REQUEST_URL", "https://actions.example/token")])]
#[case::neither(&[])]
fn oidc_github_requires_both_actions_variables(#[case] pairs: &[(&str, &str)]) {
    let environment = environment(pairs);
    let error = base_lookup()
        .get_secret(&environment, "oidc/github/audience", None)
        .expect_err("missing Actions variables are an error");
    assert_eq!(
        error.to_string(),
        "ACTIONS_ID_TOKEN_REQUEST_URL or ACTIONS_ID_TOKEN_REQUEST_TOKEN not found in environment"
    );
}

#[test]
fn oidc_github_requests_the_actions_endpoint() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, http) = scripted_lookup(vec![answer("{\"value\":\"gh-token\"}")], &clock);
    let value = lookup
        .get_secret(&actions_environment(), "oidc/github/audience", None)
        .expect("the Github provider should succeed");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("gh-token"));
    let requests = http.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.url, "https://actions.example/token");
    assert_eq!(
        request.query,
        vec![("audience".to_owned(), "audience".to_owned())]
    );
    assert_eq!(
        request.headers,
        vec![
            (
                "Authorization".to_owned(),
                "Bearer request-token".to_owned()
            ),
            (
                "Accept".to_owned(),
                "application/json; api-version=2.0".to_owned()
            ),
        ]
    );
    assert_eq!(request.timeouts, OidcHttpTimeouts::DEFAULT);
}

#[test]
fn oidc_github_failure_status_is_an_error() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, _http) = scripted_lookup(
        vec![Reply::Answer {
            status: 401,
            body: Vec::new(),
        }],
        &clock,
    );
    let error = lookup
        .get_secret(&actions_environment(), "oidc/github/audience", None)
        .expect_err("a non-200 status is an error");
    assert_eq!(error.to_string(), "Github OIDC provider failed");
}

#[test]
fn oidc_github_checks_the_environment_before_the_cache() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, http) = scripted_lookup(vec![answer("{\"value\":\"gh-token\"}")], &clock);
    lookup
        .get_secret(&actions_environment(), "oidc/github/audience", None)
        .expect("the first read should succeed");
    let error = lookup
        .get_secret(&BTreeMap::new(), "oidc/github/audience", None)
        .expect_err("the environment check comes before the cache");
    assert_eq!(
        error.to_string(),
        "ACTIONS_ID_TOKEN_REQUEST_URL or ACTIONS_ID_TOKEN_REQUEST_TOKEN not found in environment"
    );
    assert_eq!(http.requests().len(), 1);
}

#[test]
fn oidc_github_caches_a_token_for_295_seconds() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, http) = scripted_lookup(
        vec![
            answer("{\"value\":\"gh-token\"}"),
            answer("{\"value\":\"second-token\"}"),
        ],
        &clock,
    );
    lookup
        .get_secret(&actions_environment(), "oidc/github/audience", None)
        .expect("the first read should succeed");
    clock.advance(294);
    let cached = lookup
        .get_secret(&actions_environment(), "oidc/github/audience", None)
        .expect("the cached read should succeed");
    assert_eq!(text_value(cached.as_ref()).as_deref(), Some("gh-token"));
    assert_eq!(http.requests().len(), 1);
    clock.advance(2);
    let refreshed = lookup
        .get_secret(&actions_environment(), "oidc/github/audience", None)
        .expect("the expired read should succeed");
    assert_eq!(
        text_value(refreshed.as_ref()).as_deref(),
        Some("second-token")
    );
    assert_eq!(http.requests().len(), 2);
}

#[rstest]
#[case::missing("{}")]
#[case::null("{\"value\":null}")]
fn oidc_github_missing_value_returns_none(#[case] body: &str) {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, http) = scripted_lookup(vec![answer(body), answer(body)], &clock);
    assert!(
        lookup
            .get_secret(&actions_environment(), "oidc/github/audience", None)
            .expect("a missing value is not an error")
            .is_none()
    );
    assert!(
        lookup
            .get_secret(&actions_environment(), "oidc/github/audience", None)
            .expect("a None token is never cached")
            .is_none()
    );
    assert_eq!(http.requests().len(), 2);
}

#[rstest]
#[case::array("[]")]
#[case::array_with_token("[\"gh-token\"]")]
#[case::number_value("{\"value\":5}")]
#[case::text_body("\"gh-token\"")]
#[case::truncated("{\"value\":\"gh-tok")]
fn oidc_github_invalid_document_is_an_error(#[case] body: &str) {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (lookup, _http) = scripted_lookup(vec![answer(body)], &clock);
    let error = lookup
        .get_secret(&actions_environment(), "oidc/github/audience", None)
        .expect_err("an invalid token document is an error");
    assert_eq!(
        error.to_string(),
        "Github OIDC token response is not a token document"
    );
}

#[test]
fn oidc_azure_reads_the_federated_token_file_outside_the_allowlist() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("azure-token");
    fs::write(&path, "azure-file-token\n").expect("the token file");
    let environment = environment(&[(
        "AZURE_FEDERATED_TOKEN_FILE",
        path.to_str().expect("a UTF-8 path"),
    )]);
    let value = base_lookup()
        .get_secret(&environment, "oidc/azure/audience", None)
        .expect("the Azure file provider should succeed");
    assert_eq!(
        text_value(value.as_ref()).as_deref(),
        Some("azure-file-token\n")
    );
}

#[test]
fn oidc_azure_without_the_file_uses_the_token_provider() {
    let (logs, _guard) = capture_logs();
    let tokens = Arc::new(FakeAzureTokens::with(AzureReply::Token("azure-ad-token")));
    let mut lookup = base_lookup();
    let tokens_handle = Arc::clone(&tokens);
    lookup.azure_tokens = tokens_handle;
    let value = lookup
        .get_secret(&BTreeMap::new(), "oidc/azure/scope-audience", None)
        .expect("the Azure token provider should succeed");
    assert_eq!(
        text_value(value.as_ref()).as_deref(),
        Some("azure-ad-token")
    );
    assert_eq!(tokens.scopes(), vec!["scope-audience".to_owned()]);
    assert!(logs.text().contains(
        "AZURE_FEDERATED_TOKEN_FILE not found in environment will use Azure AD token provider"
    ));
}

#[test]
fn oidc_azure_none_token_is_a_wrapped_error() {
    let (logs, _guard) = capture_logs();
    let mut lookup = base_lookup();
    let tokens = Arc::new(FakeAzureTokens::with(AzureReply::None));
    let tokens_handle = Arc::clone(&tokens);
    lookup.azure_tokens = tokens_handle;
    let error = lookup
        .get_secret(&BTreeMap::new(), "oidc/azure/audience", None)
        .expect_err("a None token is an error");
    assert_eq!(
        error.to_string(),
        "Azure OIDC provider failed: Azure OIDC provider returned None token"
    );
    assert!(
        logs.text()
            .contains("Azure OIDC provider failed: Azure OIDC provider returned None token")
    );
}

#[test]
fn oidc_azure_provider_error_is_wrapped() {
    let mut lookup = base_lookup();
    let tokens = Arc::new(FakeAzureTokens::with(AzureReply::Failure("boom")));
    let tokens_handle = Arc::clone(&tokens);
    lookup.azure_tokens = tokens_handle;
    let error = lookup
        .get_secret(&BTreeMap::new(), "oidc/azure/audience", None)
        .expect_err("a provider error is an error");
    assert_eq!(error.to_string(), "Azure OIDC provider failed: boom");
}

#[test]
fn oidc_file_reads_a_path_inside_the_allowed_directories() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("token");
    fs::write(&path, "file-token").expect("the token file");
    let environment = environment(&[(
        "LITELLM_OIDC_ALLOWED_CREDENTIAL_DIRS",
        directory.path().to_str().expect("a UTF-8 path"),
    )]);
    let secret_name = format!("oidc/file/{}", path.to_str().expect("a UTF-8 path"));
    let value = base_lookup()
        .get_secret(&environment, &secret_name, None)
        .expect("the file provider should succeed");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("file-token"));
}

#[test]
fn oidc_file_outside_the_allowed_directories_is_an_error() {
    let allowed = tempfile::tempdir().expect("a temporary directory");
    let outside = tempfile::tempdir().expect("a temporary directory");
    let path = outside.path().join("token");
    fs::write(&path, "file-token").expect("the token file");
    let environment = environment(&[(
        "LITELLM_OIDC_ALLOWED_CREDENTIAL_DIRS",
        allowed.path().to_str().expect("a UTF-8 path"),
    )]);
    let secret_name = format!("oidc/file/{}", path.to_str().expect("a UTF-8 path"));
    let error = base_lookup()
        .get_secret(&environment, &secret_name, None)
        .expect_err("a path outside the allowlist is an error");
    assert!(
        error
            .to_string()
            .contains("outside the allowed credential directories")
    );
}

#[test]
fn oidc_env_reads_the_named_variable() {
    let environment = environment(&[("OIDC_TOKEN", "env-token")]);
    let value = base_lookup()
        .get_secret(&environment, "oidc/env/OIDC_TOKEN", None)
        .expect("the env provider should succeed");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("env-token"));
}

#[test]
fn oidc_env_missing_variable_is_an_error() {
    let error = base_lookup()
        .get_secret(&BTreeMap::new(), "oidc/env/OIDC_TOKEN", None)
        .expect_err("a missing variable is an error");
    assert_eq!(
        error.to_string(),
        "Environment variable OIDC_TOKEN not found"
    );
}

#[test]
fn oidc_env_path_reads_the_file_named_by_the_variable() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("token-file");
    fs::write(&path, "path-token").expect("the token file");
    let environment = environment(&[("OIDC_TOKEN_FILE", path.to_str().expect("a UTF-8 path"))]);
    let value = base_lookup()
        .get_secret(&environment, "oidc/env_path/OIDC_TOKEN_FILE", None)
        .expect("the env_path provider should succeed");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("path-token"));
}

#[test]
fn oidc_env_path_missing_variable_is_an_error() {
    let error = base_lookup()
        .get_secret(&BTreeMap::new(), "oidc/env_path/OIDC_TOKEN_FILE", None)
        .expect_err("a missing variable is an error");
    assert_eq!(
        error.to_string(),
        "Environment variable OIDC_TOKEN_FILE not found"
    );
}

#[rstest]
#[case::unknown("oidc/wat/audience")]
#[case::empty_provider("oidc//audience")]
fn oidc_unsupported_provider_is_an_error(#[case] secret_name: &str) {
    let error = base_lookup()
        .get_secret(&BTreeMap::new(), secret_name, None)
        .expect_err("an unsupported provider is an error");
    assert_eq!(error.to_string(), "Unsupported OIDC provider");
}

#[test]
fn oidc_missing_audience_is_an_error() {
    let error = base_lookup()
        .get_secret(&BTreeMap::new(), "oidc/google", None)
        .expect_err("a provider without an audience is an error");
    assert_eq!(
        error.to_string(),
        "OIDC secret name needs a provider and an audience"
    );
}

#[test]
fn oidc_only_the_leading_prefix_is_removed() {
    let environment = environment(&[("token/oidc/name", "kept")]);
    let value = base_lookup()
        .get_secret(&environment, "oidc/env/token/oidc/name", None)
        .expect("a later oidc segment belongs to the audience");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("kept"));
}

#[test]
fn oidc_resolves_after_the_os_environ_prefix_is_removed() {
    let environment = environment(&[("OIDC_TOKEN", "env-token")]);
    let value = base_lookup()
        .get_secret(&environment, "os.environ/oidc/env/OIDC_TOKEN", None)
        .expect("the prefixed OIDC name should resolve");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("env-token"));
}
