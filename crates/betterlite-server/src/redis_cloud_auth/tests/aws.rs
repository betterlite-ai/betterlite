#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aws_static_credentials_resolve_to_the_configured_keys() {
    let resolver = Arc::new(SdkAwsCredentials {
        runtime: Handle::current(),
        provider: static_aws_provider(),
    });
    let credentials = run_blocking(move || AwsCredentialsResolver::resolve(resolver.as_ref()))
        .await
        .unwrap();
    assert_eq!(credentials.access_key_id(), "AKIAFIXTURE");
    assert_eq!(credentials.secret_access_key(), "fixture-secret");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aws_a_provider_failure_maps_to_the_credentials_error() {
    let provider = SharedCredentialsProvider::new(provide_credentials_fn(|| async {
        Err(CredentialsError::not_loaded("no credentials in the test"))
    }));
    let resolver = Arc::new(SdkAwsCredentials {
        runtime: Handle::current(),
        provider,
    });
    let error = run_blocking(move || AwsCredentialsResolver::resolve(resolver.as_ref()))
        .await
        .unwrap_err();
    assert!(matches!(error, ElastiCacheSigningError::Credentials { .. }));
    assert!(error.to_string().contains("failed to resolve AWS credentials"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aws_static_credentials_give_a_signed_token() {
    let resolver = Arc::new(SdkAwsCredentials {
        runtime: Handle::current(),
        provider: static_aws_provider(),
    });
    let (user_name, signed) = run_blocking(move || {
        ElastiCacheIamProvider {
            settings: aws_settings(),
        }
        .get_credentials(resolver.as_ref(), SystemTime::UNIX_EPOCH)
    })
    .await
    .unwrap();
    assert_eq!(user_name, "app");
    let url = signed.expose_secret();
    assert!(url.starts_with("cache.example/?"));
    assert!(url.contains("Action=connect"));
    assert!(url.contains("User=app"));
    assert!(url.contains("X-Amz-Signature="));
    assert!(url.contains("X-Amz-Expires=900"));
}
