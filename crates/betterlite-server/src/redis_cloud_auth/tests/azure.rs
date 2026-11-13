fn azure_fetcher(settings: AzureSettings, grants: Arc<dyn AzureGrants>) -> AzureRedisTokens {
    AzureRedisTokens { settings, grants }
}

const LIVE_TOKEN: &str = r#"{"access_token":"fixture-token","expires_in":3600}"#;
const EXPIRED_TOKEN: &str = r#"{"access_token":"fixture-token","expires_in":0}"#;

fn production_io(token_body: &'static str, rounds: usize) -> Arc<FakeIo> {
    let openid = r#"{"issuer":"https://login.microsoftonline.com/tenant/v2.0","authorization_endpoint":"https://login.microsoftonline.com/tenant/oauth2/v2.0/authorize","token_endpoint":"https://login.microsoftonline.com/tenant/oauth2/v2.0/token"}"#;
    let mut replies = Vec::new();
    for _round in 0..rounds {
        replies.push(Reply {
            url_contains: "well-known/openid-configuration",
            status: 200,
            body: openid,
        });
        replies.push(Reply {
            url_contains: "/oauth2/v2.0/token",
            status: 200,
            body: token_body,
        });
    }
    Arc::new(FakeIo::new(replies))
}

fn production_grants(
    io: &Arc<FakeIo>,
    chain_environment: ChainEnvironment,
) -> SdkAzureGrants<FakeIo> {
    let clock = Arc::new(ProductionClock);
    let oidc_transport: Arc<dyn betterlite_azure::oidc_exchange::transport::Transport> =
        Arc::clone(io);
    let oidc_clock: Arc<dyn betterlite_azure::oidc_exchange::transport::Clock> = Arc::clone(&clock);
    let exchange = AzureOidcExchange::new(
        OidcSettings::default(),
        BTreeMap::new(),
        oidc_transport,
        oidc_clock,
    )
    .unwrap();
    SdkAzureGrants::<FakeIo> {
        http: Arc::clone(io),
        clock,
        files: StdFiles,
        ids: Arc::new(RandomIds),
        token_files: FsTokenFiles,
        scheduler: ThreadScheduler {
            runtime: Handle::current(),
        },
        host: ProductionHost {
            environment: BTreeMap::new(),
        },
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
        environment: ManagedIdentityEnvironment::default(),
        chain_environment,
        exchange,
        authority_host: None,
        client_secret_settings: ClientSecretSettings::default(),
        client_secrets: Mutex::new(None),
    }
}

#[test]
fn azure_client_secret_kind_uses_the_client_secret_grant() {
    let grants = Arc::new(RecordingGrants::new("azure-token"));
    let fetcher = azure_fetcher(
        azure_settings(AzureCredential::ClientSecret),
        Arc::clone(&grants) as Arc<dyn AzureGrants>,
    );
    let token = fetcher
        .fetch_token(AzureCredential::ClientSecret, AZURE_REDIS_SCOPE)
        .unwrap();
    assert_eq!(token.token.expose_secret(), "azure-token");
    assert_eq!(
        grants.calls(),
        vec![GrantCall::ClientSecret {
            tenant_id: "tenant".to_owned(),
            client_id: "client".to_owned(),
            scope: AZURE_REDIS_SCOPE.to_owned(),
        }]
    );
}

#[test]
fn azure_managed_identity_kind_uses_the_managed_identity_grant() {
    let grants = Arc::new(RecordingGrants::new("azure-token"));
    let fetcher = azure_fetcher(
        azure_settings(AzureCredential::ManagedIdentity),
        Arc::clone(&grants) as Arc<dyn AzureGrants>,
    );
    let token = fetcher
        .fetch_token(AzureCredential::ManagedIdentity, AZURE_REDIS_SCOPE)
        .unwrap();
    assert_eq!(token.token.expose_secret(), "azure-token");
    assert_eq!(
        grants.calls(),
        vec![GrantCall::ManagedIdentity {
            client_id: "client".to_owned(),
            scope: AZURE_REDIS_SCOPE.to_owned(),
        }]
    );
}

#[test]
fn azure_default_kind_uses_the_default_credential_grant() {
    let grants = Arc::new(RecordingGrants::new("azure-token"));
    let fetcher = azure_fetcher(
        azure_settings(AzureCredential::Default),
        Arc::clone(&grants) as Arc<dyn AzureGrants>,
    );
    let token = fetcher
        .fetch_token(AzureCredential::Default, AZURE_REDIS_SCOPE)
        .unwrap();
    assert_eq!(token.token.expose_secret(), "azure-token");
    assert_eq!(
        grants.calls(),
        vec![GrantCall::Default {
            scope: AZURE_REDIS_SCOPE.to_owned(),
        }]
    );
}

#[test]
fn azure_a_missing_client_secret_setting_is_an_error() {
    let mut settings = azure_settings(AzureCredential::ClientSecret);
    settings.client_secret = None;
    let fetcher = azure_fetcher(settings, Arc::new(RecordingGrants::refusing()));
    let error = fetcher
        .fetch_token(AzureCredential::ClientSecret, AZURE_REDIS_SCOPE)
        .unwrap_err();
    assert!(matches!(
        error,
        AzureGrantError::MissingSettings {
            fields: "client_secret",
            ..
        }
    ));
}

#[test]
fn azure_a_grant_failure_is_reported() {
    let fetcher = azure_fetcher(
        azure_settings(AzureCredential::ManagedIdentity),
        Arc::new(RecordingGrants::refusing()),
    );
    let error = fetcher
        .fetch_token(AzureCredential::ManagedIdentity, AZURE_REDIS_SCOPE)
        .unwrap_err();
    assert!(matches!(
        error,
        AzureGrantError::Grant {
            kind: "managed identity",
            ..
        }
    ));
}

#[test]
fn azure_the_fetcher_keeps_the_same_grant_across_calls() {
    let grants = Arc::new(RecordingGrants::new("azure-token"));
    let fetcher = azure_fetcher(
        azure_settings(AzureCredential::Default),
        Arc::clone(&grants) as Arc<dyn AzureGrants>,
    );
    fetcher
        .fetch_token(AzureCredential::Default, AZURE_REDIS_SCOPE)
        .unwrap();
    fetcher
        .fetch_token(AzureCredential::Default, AZURE_REDIS_SCOPE)
        .unwrap();
    assert_eq!(grants.calls().len(), 2);
    assert_eq!(Arc::strong_count(&grants), 2);
}

#[test]
fn azure_ad_provider_returns_the_username_and_token() {
    let provider = AzureAdCredentialProvider {
        credential: AzureCredential::ManagedIdentity,
        fetcher: Arc::new(azure_fetcher(
            azure_settings(AzureCredential::ManagedIdentity),
            Arc::new(RecordingGrants::new("azure-token")),
        )),
        username: Some("redis-user".to_owned()),
    };
    let (username, token) = provider.get_credentials().unwrap();
    assert_eq!(username.as_deref(), Some("redis-user"));
    assert_eq!(token.expose_secret(), "azure-token");
}

async fn two_client_secret_calls(io: &Arc<FakeIo>) -> (SecretString, SecretString) {
    let grants = production_grants(io, ChainEnvironment::default());
    tokio::task::spawn_blocking(move || {
        let client_secret = SecretString::from("secret".to_owned());
        let first = grants
            .client_secret("tenant", "client", &client_secret, AZURE_REDIS_SCOPE)
            .unwrap();
        let second = grants
            .client_secret("tenant", "client", &client_secret, AZURE_REDIS_SCOPE)
            .unwrap();
        (first, second)
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_the_client_secret_provider_is_kept_between_fetches() {
    let io = production_io(LIVE_TOKEN, 1);
    let (first, second) = two_client_secret_calls(&io).await;
    assert_eq!(first.expose_secret(), "fixture-token");
    assert_eq!(second.expose_secret(), "fixture-token");
    assert_eq!(io.requests().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_an_expired_token_is_fetched_again() {
    let io = production_io(EXPIRED_TOKEN, 2);
    let (first, second) = two_client_secret_calls(&io).await;
    assert_eq!(first.expose_secret(), "fixture-token");
    assert_eq!(second.expose_secret(), "fixture-token");
    assert_eq!(io.requests().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_the_managed_identity_grant_reports_an_unconfigured_environment() {
    let grants = production_grants(&production_io(LIVE_TOKEN, 1), ChainEnvironment::default());
    let error = tokio::task::spawn_blocking(move || {
        grants
            .managed_identity("client", AZURE_REDIS_SCOPE)
            .unwrap_err()
    })
    .await
    .unwrap();
    assert!(matches!(
        error,
        AzureGrantError::Grant {
            kind: "managed identity",
            ..
        }
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_the_default_credential_reports_an_exhausted_chain() {
    let grants = production_grants(&production_io(LIVE_TOKEN, 1), ChainEnvironment::default());
    let error = tokio::task::spawn_blocking(move || {
        grants.default_credential(AZURE_REDIS_SCOPE).unwrap_err()
    })
    .await
    .unwrap();
    assert!(matches!(error, AzureGrantError::DefaultCredential { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_two_default_fetches_send_one_token_request() {
    let io = production_io(LIVE_TOKEN, 1);
    let grants = production_grants(
        &io,
        ChainEnvironment {
            client_id: Some("client".to_owned()),
            tenant_id: Some("tenant".to_owned()),
            client_secret: Some(SecretString::from("secret".to_owned())),
            token_credentials: "environmentcredential".to_owned(),
            scope: None,
            credential: None,
            username: None,
            password: None,
            federated_token_file: None,
            certificate_path: None,
            certificate_password: None,
            client_certificate_path: None,
            client_certificate_password: None,
            client_send_certificate_chain: false,
            authority_host: None,
        },
    );
    let (first, second) = tokio::task::spawn_blocking(move || {
        let first = grants.default_credential(AZURE_REDIS_SCOPE).unwrap();
        let second = grants.default_credential(AZURE_REDIS_SCOPE).unwrap();
        (first, second)
    })
    .await
    .unwrap();
    assert_eq!(first.expose_secret(), "fixture-token");
    assert_eq!(second.expose_secret(), "fixture-token");
    assert_eq!(io.requests().len(), 2);
}
