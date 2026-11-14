const SERVICE_ACCOUNT: &str = "projects/-/serviceAccounts/sa@project.iam.gserviceaccount.com";

fn gcp_fetcher(base: String, adc: Arc<dyn AdcTokenSource>) -> Arc<IamTokenFetcher> {
    Arc::new(IamTokenFetcher {
        runtime: Handle::current(),
        http: proxied_client(),
        base,
        adc,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gcp_the_request_carries_the_service_account_body_and_bearer() {
    let server = iam_server(200, r#"{"accessToken":"iam-token"}"#).await;
    let fetcher = gcp_fetcher(
        format!("{}/", server.base),
        Arc::new(FixedAdc {
            token: Some("adc-token"),
        }),
    );
    let token = run_blocking(move || GcpTokenFetcher::fetch(fetcher.as_ref(), SERVICE_ACCOUNT))
        .await
        .unwrap();
    assert_eq!(token.expose_secret(), "iam-token");
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(
        seen[0].path,
        "/v1/projects/-/serviceAccounts/sa@project.iam.gserviceaccount.com:generateAccessToken"
    );
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer adc-token"));
    assert_eq!(seen[0].content_type.as_deref(), Some("application/json"));
    assert_eq!(
        seen[0].body,
        r#"{"scope":["https://www.googleapis.com/auth/cloud-platform"]}"#
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gcp_a_non_success_status_names_the_status() {
    let server = iam_server(403, r#"{"error":"denied"}"#).await;
    let fetcher = gcp_fetcher(
        server.base.clone(),
        Arc::new(FixedAdc {
            token: Some("adc-token"),
        }),
    );
    let error = run_blocking(move || GcpTokenFetcher::fetch(fetcher.as_ref(), SERVICE_ACCOUNT))
        .await
        .unwrap_err();
    assert!(matches!(error, IamTokenError::Status { status: 403 }));
    assert_eq!(server.seen.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gcp_a_malformed_reply_is_an_error() {
    let server = iam_server(200, "not json").await;
    let fetcher = gcp_fetcher(
        server.base.clone(),
        Arc::new(FixedAdc {
            token: Some("adc-token"),
        }),
    );
    let error = run_blocking(move || GcpTokenFetcher::fetch(fetcher.as_ref(), SERVICE_ACCOUNT))
        .await
        .unwrap_err();
    assert!(matches!(error, IamTokenError::Reply { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gcp_an_adc_failure_is_reported_before_the_request() {
    let server = iam_server(200, r#"{"accessToken":"iam-token"}"#).await;
    let fetcher = gcp_fetcher(server.base.clone(), Arc::new(FixedAdc { token: None }));
    let error = run_blocking(move || GcpTokenFetcher::fetch(fetcher.as_ref(), SERVICE_ACCOUNT))
        .await
        .unwrap_err();
    assert!(matches!(error, IamTokenError::Adc { .. }));
    assert!(server.seen.lock().unwrap().is_empty());
}

#[test]
fn gcp_decode_reads_the_access_token() {
    let token = decode_iam_token(br#"{"accessToken":"ya29.fixture"}"#).unwrap();
    assert_eq!(token.expose_secret(), "ya29.fixture");
}

#[test]
fn gcp_decode_accepts_an_empty_access_token() {
    let token = decode_iam_token(br#"{"accessToken":""}"#).unwrap();
    assert_eq!(token.expose_secret(), "");
}

#[test]
fn gcp_decode_rejects_a_missing_access_token() {
    assert!(matches!(
        decode_iam_token(b"{}"),
        Err(IamTokenError::MissingToken)
    ));
}

#[test]
fn gcp_decode_rejects_a_non_string_access_token() {
    assert!(matches!(
        decode_iam_token(br#"{"accessToken":42}"#),
        Err(IamTokenError::Reply { .. })
    ));
}

#[test]
fn gcp_decode_rejects_truncated_json() {
    assert!(matches!(
        decode_iam_token(br#"{"accessToken":"ya29"#),
        Err(IamTokenError::Reply { .. })
    ));
}

#[rstest::rstest]
fn replays_the_gcp_iam_token_corpus(#[files("fuzz/corpus/gcp_iam_token/*")] path: PathBuf) {
    let body = fs::read(&path).unwrap();
    let decoded = decode_iam_token(&body);
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return;
    };
    if name.starts_with("valid-") {
        assert!(decoded.is_ok(), "{name}");
    }
    if name.starts_with("invalid-") {
        assert!(decoded.is_err(), "{name}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gcp_production_maps_an_adc_failure_to_the_source_error() {
    let source = VertexAdcTokenSource {
        runtime: Handle::current(),
        environment: AdcEnvironment {
            application_credentials: Some(PathBuf::from("/nonexistent/betterlite-adc.json")),
            project: None,
            gcloud_project: None,
            gcloud_credentials: None,
            gcloud_configuration: None,
            metadata_host: Some("127.0.0.1:9".to_owned()),
        },
        files: VertexStdFiles,
        http: GoogleAuth {
            client: proxied_client(),
            policy: HttpPolicy::default(),
        },
    };
    let token = run_blocking(move || {
        AdcTokenSource::access_token(&source, "https://www.googleapis.com/auth/cloud-platform")
    })
    .await;
    assert!(matches!(token, Err(AdcError::Adc { .. })));
}
