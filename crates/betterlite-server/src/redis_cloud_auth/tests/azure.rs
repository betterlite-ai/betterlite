fn azure_fetcher(settings: AzureSettings, grants: Arc<dyn AzureGrants>) -> AzureRedisTokens {
    AzureRedisTokens { settings, grants }
}

fn production_io() -> Arc<FakeIo> {
    let openid = r#"{"issuer":"https://login.microsoftonline.com/tenant/v2.0","authorization_endpoint":"https://login.microsoftonline.com/tenant/oauth2/v2.0/authorize","token_endpoint":"https://login.microsoftonline.com/tenant/oauth2/v2.0/token"}"#;
    Arc::new(FakeIo::new(vec![
        Reply {
            url_contains: "well-known/openid-configuration",
            status: 200,
            body: openid,
        },
        Reply {
            url_contains: "/oauth2/v2.0/token",
            status: 200,
            body: r#"{"access_token":"fixture-token","expires_in":3600}"#,
        },
    ]))
}

fn production_grants(io: &Arc<FakeIo>) -> SdkAzureGrants<FakeIo> {
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
    let transport: Arc<dyn betterlite_azure::azure_client_secret::transport::Transport> =
        Arc::clone(io);
    let client_clock: Arc<dyn betterlite_azure::azure_client_secret::transport::Clock> =
        Arc::clone(&clock);
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
        chain_environment: ChainEnvironment::default(),
        exchange,
        authority_host: None,
        client_secrets: Mutex::new(ProviderCache::new(
            transport,
            client_clock,
            ClientSecretSettings::default(),
        )),
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
    assert!(matches!(error, AzureGrantError::ManagedIdentity { .. }));
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

#[test]
fn azure_the_client_secret_credential_is_built_once() {
    let io = production_io();
    let grants = production_grants(&io);
    let client_secret = SecretString::from("secret".to_owned());
    let first = grants
        .client_secret("tenant", "client", &client_secret, AZURE_REDIS_SCOPE)
        .unwrap();
    let second = grants
        .client_secret("tenant", "client", &client_secret, AZURE_REDIS_SCOPE)
        .unwrap();
    assert_eq!(first.expose_secret(), "fixture-token");
    assert_eq!(second.expose_secret(), "fixture-token");
    assert_eq!(io.requests().len(), 2);
}
