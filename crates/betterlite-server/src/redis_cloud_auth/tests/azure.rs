fn azure_fetcher(settings: AzureSettings, grants: Arc<dyn AzureGrants>) -> AzureRedisTokens {
    AzureRedisTokens { settings, grants }
}

const LIVE_TOKEN: &str = r#"{"access_token":"fixture-token","expires_in":3600}"#;
const EXPIRED_TOKEN: &str = r#"{"access_token":"fixture-token","expires_in":0}"#;
const IMDS_FIRST: &str = r#"{"access_token":"imds-first","expires_in":3600}"#;
const IMDS_SECOND: &str = r#"{"access_token":"imds-second","expires_in":3600}"#;
const IMDS_EXPIRED: &str = r#"{"access_token":"imds-first","expires_in":0}"#;

struct StepClock {
    now: AtomicI64,
}

impl TokenClock for StepClock {
    fn now_unix_seconds(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

fn chain_token(secret: &str, expires_on: Option<i64>, refresh_on: Option<i64>) -> ChainToken {
    ChainToken {
        token: SecretString::from(secret.to_owned()),
        expires_on,
        token_type: None,
        refresh_on,
    }
}

fn failed_refresh() -> Result<ChainToken, AzureGrantError> {
    Err(AzureGrantError::Grant {
        kind: "managed identity",
        message: "the test refresh fails".to_owned(),
    })
}

// the chain holds only the managed identity link, which sends no IMDS probe
fn managed_identity_chain() -> ChainEnvironment {
    ChainEnvironment {
        client_id: None,
        tenant_id: None,
        client_secret: None,
        token_credentials: "managedidentitycredential".to_owned(),
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
    }
}

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
    Arc::new(FakeIo::new(replies, Vec::new()))
}

fn production_grants(
    io: &Arc<FakeIo>,
    chain_environment: ChainEnvironment,
) -> SdkAzureGrants<FakeIo> {
    let clock = Arc::new(ProductionClock);
    let oidc_transport: Arc<dyn OidcTransport> = Arc::<FakeIo>::clone(io);
    let oidc_clock: Arc<dyn OidcClock> = Arc::<ProductionClock>::clone(&clock);
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
        client_secrets: KeptProvider::default(),
        managed_identity_tokens: TokenCache::default(),
        default_tokens: TokenCache::default(),
    }
}

#[test]
fn azure_client_secret_kind_uses_the_client_secret_grant() {
    let grants = Arc::new(RecordingGrants::new("azure-token"));
    let fetcher = azure_fetcher(
        azure_settings(AzureCredential::ClientSecret),
        Arc::<RecordingGrants>::clone(&grants),
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
        Arc::<RecordingGrants>::clone(&grants),
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
        Arc::<RecordingGrants>::clone(&grants),
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
        Arc::<RecordingGrants>::clone(&grants),
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
    run_blocking(move || {
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
    let error = run_blocking(move || {
        grants
            .managed_identity("client", AZURE_REDIS_SCOPE)
            .unwrap_err()
    })
    .await;
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
    let error =
        run_blocking(move || grants.default_credential(AZURE_REDIS_SCOPE).unwrap_err()).await;
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
    let (first, second) = run_blocking(move || {
        let first = grants.default_credential(AZURE_REDIS_SCOPE).unwrap();
        let second = grants.default_credential(AZURE_REDIS_SCOPE).unwrap();
        (first, second)
    })
    .await;
    assert_eq!(first.expose_secret(), "fixture-token");
    assert_eq!(second.expose_secret(), "fixture-token");
    assert_eq!(io.requests().len(), 2);
}

#[test]
fn azure_token_cache_serves_the_token_until_its_refresh_time() {
    let cache = TokenCache::default();
    let clock = StepClock {
        now: AtomicI64::new(100),
    };
    let fetches = AtomicUsize::new(0);
    let fetch = || {
        let count = fetches.fetch_add(1, Ordering::SeqCst);
        Ok(chain_token(
            &format!("token-{count}"),
            Some(1000),
            Some(500),
        ))
    };
    let first = cache.get_or_fetch(&["scope"], &clock, fetch).unwrap();
    clock.now.store(499, Ordering::SeqCst);
    let second = cache.get_or_fetch(&["scope"], &clock, fetch).unwrap();
    clock.now.store(500, Ordering::SeqCst);
    let third = cache.get_or_fetch(&["scope"], &clock, fetch).unwrap();
    assert_eq!(first.token.expose_secret(), "token-0");
    assert_eq!(second.token.expose_secret(), "token-0");
    assert_eq!(third.token.expose_secret(), "token-1");
    assert_eq!(fetches.load(Ordering::SeqCst), 2);
}

#[test]
fn azure_token_cache_serves_until_expiry_without_a_refresh_time() {
    let cache = TokenCache::default();
    let clock = StepClock {
        now: AtomicI64::new(100),
    };
    let fetches = AtomicUsize::new(0);
    let fetch = || {
        let count = fetches.fetch_add(1, Ordering::SeqCst);
        Ok(chain_token(&format!("token-{count}"), Some(1000), None))
    };
    let first = cache.get_or_fetch(&["scope"], &clock, fetch).unwrap();
    clock.now.store(999, Ordering::SeqCst);
    let second = cache.get_or_fetch(&["scope"], &clock, fetch).unwrap();
    clock.now.store(1000, Ordering::SeqCst);
    let third = cache.get_or_fetch(&["scope"], &clock, fetch).unwrap();
    assert_eq!(first.token.expose_secret(), "token-0");
    assert_eq!(second.token.expose_secret(), "token-0");
    assert_eq!(third.token.expose_secret(), "token-1");
}

#[test]
fn azure_token_cache_serves_the_aging_token_when_the_refresh_fails() {
    let cache = TokenCache::default();
    let clock = StepClock {
        now: AtomicI64::new(100),
    };
    let first = cache
        .get_or_fetch(&["scope"], &clock, || {
            Ok(chain_token("aging", Some(1000), Some(500)))
        })
        .unwrap();
    clock.now.store(999, Ordering::SeqCst);
    let served = cache
        .get_or_fetch(&["scope"], &clock, failed_refresh)
        .unwrap();
    clock.now.store(1000, Ordering::SeqCst);
    let error = cache
        .get_or_fetch(&["scope"], &clock, failed_refresh)
        .unwrap_err();
    assert_eq!(first.token.expose_secret(), "aging");
    assert_eq!(served.token.expose_secret(), "aging");
    assert!(matches!(
        error,
        AzureGrantError::Grant {
            kind: "managed identity",
            ..
        }
    ));
}

#[test]
fn azure_token_cache_without_an_expiry_is_fetched_every_time() {
    let cache = TokenCache::default();
    let clock = StepClock {
        now: AtomicI64::new(100),
    };
    let fetches = AtomicUsize::new(0);
    let fetch = || {
        let count = fetches.fetch_add(1, Ordering::SeqCst);
        Ok(chain_token(&format!("token-{count}"), None, None))
    };
    let first = cache.get_or_fetch(&["scope"], &clock, fetch).unwrap();
    let second = cache.get_or_fetch(&["scope"], &clock, fetch).unwrap();
    let error = cache
        .get_or_fetch(&["scope"], &clock, failed_refresh)
        .unwrap_err();
    assert_eq!(first.token.expose_secret(), "token-0");
    assert_eq!(second.token.expose_secret(), "token-1");
    assert!(matches!(error, AzureGrantError::Grant { .. }));
}

#[test]
fn azure_token_cache_fetches_again_for_another_key() {
    let cache = TokenCache::default();
    let clock = StepClock {
        now: AtomicI64::new(100),
    };
    let one = cache
        .get_or_fetch(&["one"], &clock, || {
            Ok(chain_token("one", Some(1000), None))
        })
        .unwrap();
    let two = cache
        .get_or_fetch(&["two"], &clock, || {
            Ok(chain_token("two", Some(1000), None))
        })
        .unwrap();
    let longer = cache
        .get_or_fetch(&["two", "extra"], &clock, || {
            Ok(chain_token("longer", Some(1000), None))
        })
        .unwrap();
    let error = cache
        .get_or_fetch(&["other"], &clock, failed_refresh)
        .unwrap_err();
    assert_eq!(one.token.expose_secret(), "one");
    assert_eq!(two.token.expose_secret(), "two");
    assert_eq!(longer.token.expose_secret(), "longer");
    assert!(matches!(error, AzureGrantError::Grant { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_two_managed_identity_fetches_send_one_imds_request() {
    let io = Arc::new(FakeIo::new(Vec::new(), vec![IMDS_FIRST, IMDS_SECOND]));
    let grants = production_grants(&io, ChainEnvironment::default());
    let (first, second) = run_blocking(move || {
        let first = grants
            .managed_identity("client", AZURE_REDIS_SCOPE)
            .unwrap();
        let second = grants
            .managed_identity("client", AZURE_REDIS_SCOPE)
            .unwrap();
        (first, second)
    })
    .await;
    assert_eq!(first.expose_secret(), "imds-first");
    assert_eq!(second.expose_secret(), "imds-first");
    assert_eq!(io.requests().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_an_expired_managed_identity_token_is_fetched_again() {
    let io = Arc::new(FakeIo::new(Vec::new(), vec![IMDS_EXPIRED, IMDS_SECOND]));
    let grants = production_grants(&io, ChainEnvironment::default());
    let (first, second) = run_blocking(move || {
        let first = grants
            .managed_identity("client", AZURE_REDIS_SCOPE)
            .unwrap();
        let second = grants
            .managed_identity("client", AZURE_REDIS_SCOPE)
            .unwrap();
        (first, second)
    })
    .await;
    assert_eq!(first.expose_secret(), "imds-first");
    assert_eq!(second.expose_secret(), "imds-second");
    assert_eq!(io.requests().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_a_managed_identity_for_another_client_is_fetched_separately() {
    let io = Arc::new(FakeIo::new(Vec::new(), vec![IMDS_FIRST, IMDS_SECOND]));
    let grants = production_grants(&io, ChainEnvironment::default());
    let (first, second) = run_blocking(move || {
        let first = grants.managed_identity("one", AZURE_REDIS_SCOPE).unwrap();
        let second = grants.managed_identity("two", AZURE_REDIS_SCOPE).unwrap();
        (first, second)
    })
    .await;
    assert_eq!(first.expose_secret(), "imds-first");
    assert_eq!(second.expose_secret(), "imds-second");
    assert_eq!(io.requests().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_two_default_managed_identity_fetches_send_one_imds_request() {
    let io = Arc::new(FakeIo::new(Vec::new(), vec![IMDS_FIRST, IMDS_SECOND]));
    let grants = production_grants(&io, managed_identity_chain());
    let (first, second) = run_blocking(move || {
        let first = grants.default_credential(AZURE_REDIS_SCOPE).unwrap();
        let second = grants.default_credential(AZURE_REDIS_SCOPE).unwrap();
        (first, second)
    })
    .await;
    assert_eq!(first.expose_secret(), "imds-first");
    assert_eq!(second.expose_secret(), "imds-first");
    assert_eq!(io.requests().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_concurrent_default_fetches_send_one_imds_request() {
    let io = Arc::new(FakeIo::new(Vec::new(), vec![IMDS_FIRST, IMDS_SECOND]));
    let grants = Arc::new(production_grants(&io, managed_identity_chain()));
    let mut tasks = JoinSet::new();
    (0_u8..4).for_each(|_index| {
        let shared = Arc::clone(&grants);
        tasks.spawn_blocking(move || shared.default_credential(AZURE_REDIS_SCOPE).unwrap());
    });
    let tokens = tasks.join_all().await;
    assert_eq!(tokens.len(), 4);
    assert!(
        tokens
            .iter()
            .all(|token| token.expose_secret() == "imds-first")
    );
    assert_eq!(io.requests().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_concurrent_client_secret_fetches_share_one_provider() {
    let io = production_io(LIVE_TOKEN, 1);
    let grants = Arc::new(production_grants(&io, ChainEnvironment::default()));
    let mut tasks = JoinSet::new();
    (0_u8..4).for_each(|_index| {
        let shared = Arc::clone(&grants);
        tasks.spawn_blocking(move || {
            let client_secret = SecretString::from("secret".to_owned());
            shared
                .client_secret("tenant", "client", &client_secret, AZURE_REDIS_SCOPE)
                .unwrap()
        });
    });
    let tokens = tasks.join_all().await;
    assert_eq!(tokens.len(), 4);
    assert!(
        tokens
            .iter()
            .all(|token| token.expose_secret() == "fixture-token")
    );
    assert_eq!(io.requests().len(), 2);
}
