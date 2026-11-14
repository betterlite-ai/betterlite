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
    let dependencies = source_deps(
        Arc::new(FixedAdc { token: None }),
        Arc::new(RecordingGrants::refusing()),
        static_aws_provider(),
    );
    assert!(cloud_redis_auth(&settings, &dependencies).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_refresh_after_is_3300_for_gcp() {
    let settings = sourced_settings(gcp_provider());
    let dependencies = source_deps(
        Arc::new(FixedAdc { token: None }),
        Arc::new(RecordingGrants::refusing()),
        static_aws_provider(),
    );
    let source = cloud_redis_auth(&settings, &dependencies).unwrap();
    assert_eq!(source.refresh_after(), Duration::from_mins(55));
    assert_eq!(GCP_REFRESH_AFTER, Duration::from_mins(55));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_refresh_after_is_300_for_azure() {
    let settings = sourced_settings(azure_provider(AzureCredential::ManagedIdentity));
    let dependencies = source_deps(
        Arc::new(FixedAdc { token: None }),
        Arc::new(RecordingGrants::refusing()),
        static_aws_provider(),
    );
    let source = cloud_redis_auth(&settings, &dependencies).unwrap();
    assert_eq!(source.refresh_after(), Duration::from_secs(300));
    assert_eq!(AZURE_REFRESH_AFTER, Duration::from_secs(300));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_refresh_after_is_840_for_aws() {
    let settings = sourced_settings(aws_provider());
    let dependencies = source_deps(
        Arc::new(FixedAdc { token: None }),
        Arc::new(RecordingGrants::refusing()),
        static_aws_provider(),
    );
    let source = cloud_redis_auth(&settings, &dependencies).unwrap();
    assert_eq!(source.refresh_after(), Duration::from_mins(14));
    assert_eq!(AWS_REFRESH_AFTER, Duration::from_mins(14));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_azure_returns_the_username_and_token() {
    let settings = sourced_settings(azure_provider(AzureCredential::ManagedIdentity));
    let dependencies = source_deps(
        Arc::new(FixedAdc { token: None }),
        Arc::new(RecordingGrants::new("azure-token")),
        static_aws_provider(),
    );
    let source = cloud_redis_auth(&settings, &dependencies).unwrap();
    let credentials = source.fetch().unwrap();
    assert_eq!(credentials.username.as_deref(), Some("redis-user"));
    assert_eq!(credentials.password.expose_secret(), "azure-token");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_aws_returns_the_user_name_and_signed_token() {
    let settings = sourced_settings(aws_provider());
    let dependencies = source_deps(
        Arc::new(FixedAdc { token: None }),
        Arc::new(RecordingGrants::refusing()),
        static_aws_provider(),
    );
    let source = cloud_redis_auth(&settings, &dependencies).unwrap();
    let credentials = run_blocking(move || source.fetch()).await.unwrap();
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
    let settings = sourced_settings(gcp_provider());
    let dependencies = source_deps(
        Arc::new(FixedAdc { token: None }),
        Arc::new(RecordingGrants::refusing()),
        static_aws_provider(),
    );
    let source = cloud_redis_auth(&settings, &dependencies).unwrap();
    let error = source.fetch().err().unwrap();
    match error {
        RedisCredentialError::Fetch(cause) => assert!(
            cause
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
    let settings = sourced_settings(aws_provider());
    let dependencies = source_deps(
        Arc::new(FixedAdc { token: None }),
        Arc::new(RecordingGrants::refusing()),
        failing,
    );
    let source = cloud_redis_auth(&settings, &dependencies).unwrap();
    let error = run_blocking(move || source.fetch()).await.err().unwrap();
    match error {
        RedisCredentialError::Fetch(cause) => {
            assert!(cause.to_string().contains("failed to resolve AWS credentials"));
        }
    }
}
