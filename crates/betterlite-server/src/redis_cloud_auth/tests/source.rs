fn source_deps(
    adc: Arc<dyn AdcTokenSource>,
    azure: Arc<dyn AzureGrants>,
    aws: SharedCredentialsProvider,
) -> CloudRedisAuthDeps {
    deps(Handle::current(), adc, azure, aws)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_without_a_provider_returns_none() {
    let settings = Settings::default();
    assert!(
        cloud_redis_auth(
            &settings,
            &source_deps(
                Arc::new(FixedAdc { token: None }),
                Arc::new(FailingGrants),
                static_aws_provider(),
            )
        )
        .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_refresh_after_is_3300_for_gcp() {
    let source = cloud_redis_auth(
        &gcp_settings(),
        &source_deps(
            Arc::new(FixedAdc { token: None }),
            Arc::new(FailingGrants),
            static_aws_provider(),
        ),
    )
    .unwrap();
    assert_eq!(source.refresh_after(), Duration::from_secs(3300));
    assert_eq!(GCP_REFRESH_AFTER, Duration::from_secs(3300));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_refresh_after_is_300_for_azure() {
    let source = cloud_redis_auth(
        &azure_sourced_settings(AzureCredential::ManagedIdentity),
        &source_deps(
            Arc::new(FixedAdc { token: None }),
            Arc::new(FailingGrants),
            static_aws_provider(),
        ),
    )
    .unwrap();
    assert_eq!(source.refresh_after(), Duration::from_secs(300));
    assert_eq!(AZURE_REFRESH_AFTER, Duration::from_secs(300));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_refresh_after_is_840_for_aws() {
    let source = cloud_redis_auth(
        &aws_sourced_settings(),
        &source_deps(
            Arc::new(FixedAdc { token: None }),
            Arc::new(FailingGrants),
            static_aws_provider(),
        ),
    )
    .unwrap();
    assert_eq!(source.refresh_after(), Duration::from_secs(840));
    assert_eq!(AWS_REFRESH_AFTER, Duration::from_secs(840));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_azure_returns_the_username_and_token() {
    let source = cloud_redis_auth(
        &azure_sourced_settings(AzureCredential::ManagedIdentity),
        &source_deps(
            Arc::new(FixedAdc { token: None }),
            Arc::new(RecordingGrants::new("azure-token")),
            static_aws_provider(),
        ),
    )
    .unwrap();
    let credentials = source.fetch().unwrap();
    assert_eq!(credentials.username.as_deref(), Some("redis-user"));
    assert_eq!(credentials.password.expose_secret(), "azure-token");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_aws_returns_the_user_name_and_signed_token() {
    let source = cloud_redis_auth(
        &aws_sourced_settings(),
        &source_deps(
            Arc::new(FixedAdc { token: None }),
            Arc::new(FailingGrants),
            static_aws_provider(),
        ),
    )
    .unwrap();
    let credentials = tokio::task::spawn_blocking(move || source.fetch())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(credentials.username.as_deref(), Some("app"));
    assert!(
        credentials
            .password
            .expose_secret()
            .contains("X-Amz-Signature=")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_gcp_maps_a_fetch_failure_to_the_credential_error() {
    let source = cloud_redis_auth(
        &gcp_settings(),
        &source_deps(
            Arc::new(FixedAdc { token: None }),
            Arc::new(FailingGrants),
            static_aws_provider(),
        ),
    )
    .unwrap();
    let error = source.fetch().unwrap_err();
    match error {
        RedisCredentialError::Fetch(source) => assert!(
            source
                .to_string()
                .contains("application default credentials")
        ),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_aws_maps_a_fetch_failure_to_the_credential_error() {
    let failing = SharedCredentialsProvider::new(provide_credentials_fn(|| async {
        Err(CredentialsError::not_loaded("no credentials in the test"))
    }));
    let source = cloud_redis_auth(
        &aws_sourced_settings(),
        &source_deps(
            Arc::new(FixedAdc { token: None }),
            Arc::new(FailingGrants),
            failing,
        ),
    )
    .unwrap();
    let error = tokio::task::spawn_blocking(move || source.fetch())
        .await
        .unwrap()
        .unwrap_err();
    match error {
        RedisCredentialError::Fetch(source) => {
            assert!(source.to_string().contains("no credentials in the test"));
        }
    }
}
