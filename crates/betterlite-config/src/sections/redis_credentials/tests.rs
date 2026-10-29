#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use aws_credential_types::Credentials;

use super::aws::{AwsCredentialsResolver, AwsSettings, ElastiCacheIamProvider};
use super::azure::{
    AZURE_REDIS_SCOPE, AzureAdCredentialProvider, AzureCredential, AzureToken, AzureTokenError,
    AzureTokenFetcher,
};
use super::gcp::{Clock, GcpIamCredentialProvider, GcpTokenCache, GcpTokenError, GcpTokenFetcher};
use super::{Provider, ProviderWarning};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: BTreeMap<String, String>) -> super::Settings {
    ConfigLoader {
        lookup: environment,
    }
    .load(source)
    .unwrap()
    .sections
    .redis_credentials
}

#[test]
fn resolves_gcp_from_environment_with_precedence_over_azure() {
    let mut environment = BTreeMap::new();
    environment.insert(
        "REDIS_GCP_SERVICE_ACCOUNT".to_owned(),
        "projects/-/serviceAccounts/redis".to_owned(),
    );
    environment.insert("REDIS_AZURE_AD_TOKEN".to_owned(), "true".to_owned());
    let settings = load("{}", environment);
    let Some(Provider::Gcp(gcp)) = &settings.provider else {
        panic!("expected a gcp provider, got {:?}", settings.provider);
    };
    assert_eq!(gcp.token_ttl_seconds, 3300);
    assert_eq!(settings.warnings, vec![ProviderWarning::GcpOutranksAzure]);
    assert!(settings.environment_snapshot["REDIS_GCP_SERVICE_ACCOUNT"]);
}

#[test]
fn empty_gcp_service_account_in_config_falls_back_to_unset() {
    let source = "litellm_settings:\n  cache_params:\n    gcp_service_account: \"\"\n";
    let settings = load(source, BTreeMap::new());
    assert!(settings.provider.is_none());
}

#[test]
fn azure_ad_token_must_be_exactly_true_to_enable_azure() {
    let mut environment = BTreeMap::new();
    environment.insert("AZURE_CLIENT_ID".to_owned(), "client".to_owned());
    let settings = load("{}", environment);
    assert!(settings.provider.is_none());
}

#[rstest]
#[case("true", true)]
#[case("TRUE", true)]
#[case("false", false)]
#[case("client-id-only", false)]
fn azure_ad_token_gates_on_strict_true(#[case] token: &str, #[case] expect_enabled: bool) {
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AZURE_AD_TOKEN".to_owned(), token.to_owned());
    environment.insert("AZURE_CLIENT_ID".to_owned(), "client".to_owned());
    let settings = load("{}", environment);
    assert_eq!(
        matches!(settings.provider, Some(Provider::Azure(_))),
        expect_enabled
    );
}

#[rstest]
#[case(
    Some("client"),
    Some("tenant"),
    Some("secret"),
    AzureCredential::ClientSecret
)]
#[case(Some("client"), None, None, AzureCredential::ManagedIdentity)]
#[case(None, None, None, AzureCredential::Default)]
fn selects_azure_credential_kind_from_config(
    #[case] client_id: Option<&str>,
    #[case] tenant_id: Option<&str>,
    #[case] client_secret: Option<&str>,
    #[case] expected: AzureCredential,
) {
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AZURE_AD_TOKEN".to_owned(), "true".to_owned());
    if let Some(value) = client_id {
        environment.insert("AZURE_CLIENT_ID".to_owned(), value.to_owned());
    }
    if let Some(value) = tenant_id {
        environment.insert("AZURE_TENANT_ID".to_owned(), value.to_owned());
    }
    if let Some(value) = client_secret {
        environment.insert("AZURE_CLIENT_SECRET".to_owned(), value.to_owned());
    }
    let settings = load("{}", environment);
    let Some(Provider::Azure(azure)) = settings.provider else {
        panic!("expected an azure provider");
    };
    assert_eq!(azure.credential, expected);
}

#[test]
fn azure_username_survives_with_provider_active() {
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AZURE_AD_TOKEN".to_owned(), "true".to_owned());
    environment.insert("AZURE_CLIENT_ID".to_owned(), "client".to_owned());
    environment.insert("REDIS_USERNAME".to_owned(), "app".to_owned());
    let settings = load("{}", environment);
    let Some(Provider::Azure(azure)) = settings.provider else {
        panic!("expected an azure provider");
    };
    assert_eq!(azure.username, Some("app".to_owned()));
    assert_eq!(settings.username, Some("app".to_owned()));
}

#[test]
fn empty_redis_username_with_azure_active_stays_unset() {
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AZURE_AD_TOKEN".to_owned(), "true".to_owned());
    environment.insert("AZURE_CLIENT_ID".to_owned(), "client".to_owned());
    environment.insert("REDIS_USERNAME".to_owned(), String::new());
    let settings = load("{}", environment);
    let Some(Provider::Azure(azure)) = settings.provider else {
        panic!("expected an azure provider");
    };
    assert_eq!(azure.username, None);
    assert_eq!(settings.username, None);
}

#[test]
fn gcp_outranks_aws_and_warns() {
    let mut environment = BTreeMap::new();
    environment.insert(
        "REDIS_GCP_SERVICE_ACCOUNT".to_owned(),
        "projects/-/serviceAccounts/redis".to_owned(),
    );
    environment.insert("REDIS_AWS_IAM_AUTH".to_owned(), "true".to_owned());
    let settings = load("{}", environment);
    assert!(matches!(settings.provider, Some(Provider::Gcp(_))));
    assert_eq!(settings.warnings, vec![ProviderWarning::GcpOutranksAws]);
}

#[test]
fn azure_outranks_aws_and_warns() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n    ssl: true\n";
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AZURE_AD_TOKEN".to_owned(), "true".to_owned());
    environment.insert("AZURE_CLIENT_ID".to_owned(), "client".to_owned());
    let settings = load(source, environment);
    assert!(matches!(settings.provider, Some(Provider::Azure(_))));
    assert_eq!(settings.warnings, vec![ProviderWarning::AzureOutranksAws]);
}

#[rstest]
#[case(
    "aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    ssl: true\n",
    "aws_iam_region"
)]
#[case(
    "aws_iam_region: eu-west-1\n    aws_iam_cache_name: cache\n    ssl: true\n",
    "aws_iam_user_name"
)]
#[case(
    "aws_iam_region: eu-west-1\n    aws_iam_user_name: app\n    ssl: true\n",
    "aws_iam_cache_name"
)]
#[case(
    "aws_iam_user_name: \"\"\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n    ssl: true\n",
    "aws_iam_user_name"
)]
#[case(
    "aws_iam_user_name: app\n    aws_iam_cache_name: \"\"\n    aws_iam_region: eu-west-1\n    ssl: true\n",
    "aws_iam_cache_name"
)]
#[case(
    "aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: \"\"\n    ssl: true\n",
    "aws_iam_region"
)]
fn aws_requires_every_setting(#[case] remaining: &str, #[case] missing: &str) {
    let source =
        format!("litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    {remaining}");
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .unwrap_err();
    let ConfigError::MissingEnvironment { variable, path } = error else {
        panic!("expected a missing-environment error, got {error:?}");
    };
    assert_eq!(variable, missing);
    assert_eq!(path, format!("litellm_settings.cache_params.{missing}"));
}

#[test]
fn aws_empty_region_falls_back_to_environment() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: \"\"\n    ssl: true\n";
    let mut environment = BTreeMap::new();
    environment.insert("AWS_REGION".to_owned(), "eu-west-1".to_owned());
    let settings = load(source, environment);
    let Some(Provider::Aws(aws)) = settings.provider else {
        panic!("expected an aws provider");
    };
    assert_eq!(aws.region, "eu-west-1");
}

#[rstest]
#[case("true", true)]
#[case("\"1\"", true)]
#[case("yes", true)]
#[case("false", false)]
fn aws_serverless_accepts_text_forms(#[case] value: &str, #[case] expect_serverless: bool) {
    let source = format!(
        "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n    aws_iam_serverless: {value}\n    ssl: true\n"
    );
    let settings = load(&source, BTreeMap::new());
    let Some(Provider::Aws(aws)) = settings.provider else {
        panic!("expected an aws provider");
    };
    assert_eq!(aws.serverless, expect_serverless);
}

#[test]
fn aws_iam_serverless_config_false_overrides_environment_true() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n    aws_iam_serverless: false\n    ssl: true\n";
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AWS_IAM_SERVERLESS".to_owned(), "true".to_owned());
    let settings = load(source, environment);
    let Some(Provider::Aws(aws)) = settings.provider else {
        panic!("expected an aws provider");
    };
    assert!(!aws.serverless);
}

#[test]
fn aws_iam_auth_config_false_overrides_environment_true() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: false\n";
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AWS_IAM_AUTH".to_owned(), "true".to_owned());
    let settings = load(source, environment);
    assert!(settings.provider.is_none());
}

#[test]
fn aws_requires_tls_via_ssl_flag() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n";
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .unwrap_err();
    let ConfigError::InvalidScalar { path, value } = error else {
        panic!("expected an invalid-scalar error, got {error:?}");
    };
    assert_eq!(path, "litellm_settings.cache_params.ssl");
    assert!(!value);
}

#[test]
fn aws_requires_tls_accepts_rediss_url_without_ssl_flag() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n    url: rediss://cache.example:6380\n";
    let settings = load(source, BTreeMap::new());
    assert!(matches!(settings.provider, Some(Provider::Aws(_))));
}

#[test]
fn lowercases_aws_cache_and_requires_tls() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: Cache.EXAMPLE\n    aws_iam_region: eu-west-1\n    ssl: true\n";
    let settings = load(source, BTreeMap::new());
    let Some(Provider::Aws(aws)) = settings.provider else {
        panic!("expected an aws provider");
    };
    assert_eq!(aws.cache_name, "cache.example");
    assert_eq!(aws.token_ttl_seconds, 900);
}

#[rstest]
#[case("app", false, "cache", "Action=connect&User=app")]
#[case(
    "app",
    true,
    "cache",
    "Action=connect&User=app&ResourceType=ServerlessCache"
)]
#[case("app user", false, "cache", "Action=connect&User=app+user")]
fn connect_query_encodes_user_and_resource_type(
    #[case] user_name: &str,
    #[case] serverless: bool,
    #[case] cache_name: &str,
    #[case] expected: &str,
) {
    let aws = AwsSettings {
        user_name: user_name.to_owned(),
        cache_name: cache_name.to_owned(),
        region: "eu-west-1".to_owned(),
        serverless,
        token_ttl_seconds: 900,
    };
    assert_eq!(aws.connect_query(), expected);
}

#[test]
fn strips_userinfo_from_url_when_provider_is_active() {
    let mut environment = BTreeMap::new();
    environment.insert(
        "REDIS_GCP_SERVICE_ACCOUNT".to_owned(),
        "projects/-/serviceAccounts/redis".to_owned(),
    );
    environment.insert(
        "REDIS_URL".to_owned(),
        "redis://user:pass@cache.example:6379".to_owned(),
    );
    let settings = load("{}", environment);
    let url = settings.url.unwrap();
    let exposed = url.expose_secret();
    assert!(!exposed.contains("user"));
    assert!(!exposed.contains("pass"));
}

#[test]
fn keeps_username_and_password_when_no_provider_is_active() {
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_USERNAME".to_owned(), "app".to_owned());
    environment.insert("REDIS_PASSWORD".to_owned(), "secret".to_owned());
    let settings = load("{}", environment);
    assert_eq!(settings.username, Some("app".to_owned()));
    assert!(settings.password.is_some());
}

#[test]
fn environment_snapshot_reports_unset_names_as_false() {
    let settings = load("{}", BTreeMap::new());
    assert!(!settings.environment_snapshot["REDIS_GCP_SERVICE_ACCOUNT"]);
    assert!(!settings.environment_snapshot["AWS_REGION"]);
}

struct FakeClock {
    now: Mutex<Instant>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            now: Mutex::new(Instant::now()),
        }
    }

    fn advance(&self, duration: Duration) {
        let mut now = self.now.lock().unwrap();
        *now = now.checked_add(duration).unwrap();
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }
}

struct CountingGcpFetcher {
    calls: Mutex<Vec<()>>,
}

impl CountingGcpFetcher {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl GcpTokenFetcher for CountingGcpFetcher {
    fn fetch(
        &self,
        service_account: &str,
    ) -> Result<SecretString, Box<dyn StdError + Send + Sync + 'static>> {
        let mut calls = self.calls.lock().map_err(|_poisoned| {
            let error: Box<dyn StdError + Send + Sync> = Box::new(GcpTokenError::LockPoisoned);
            error
        })?;
        calls.push(());
        let count = calls.len();
        drop(calls);
        Ok(SecretString::from(format!(
            "{service_account}-token-{count}"
        )))
    }
}

#[test]
fn gcp_token_cache_reuses_token_within_ttl_and_refetches_after_expiry() {
    let clock = FakeClock::new();
    let fetcher = CountingGcpFetcher::new();
    let cache = GcpTokenCache::default();
    let ttl = Duration::from_mins(55);

    let first = cache
        .get_or_fetch("account-a", ttl, &clock, &fetcher)
        .unwrap();
    let second = cache
        .get_or_fetch("account-a", ttl, &clock, &fetcher)
        .unwrap();
    assert_eq!(first.expose_secret(), second.expose_secret());
    assert_eq!(fetcher.call_count(), 1);

    clock.advance(ttl + Duration::from_secs(1));
    let third = cache
        .get_or_fetch("account-a", ttl, &clock, &fetcher)
        .unwrap();
    assert_ne!(first.expose_secret(), third.expose_secret());
    assert_eq!(fetcher.call_count(), 2);
}

#[test]
fn gcp_token_cache_is_keyed_by_service_account() {
    let clock = FakeClock::new();
    let fetcher = CountingGcpFetcher::new();
    let cache = GcpTokenCache::default();
    let ttl = Duration::from_mins(55);

    let first_account = cache
        .get_or_fetch("account-a", ttl, &clock, &fetcher)
        .unwrap();
    let second_account = cache
        .get_or_fetch("account-b", ttl, &clock, &fetcher)
        .unwrap();
    assert_ne!(
        first_account.expose_secret(),
        second_account.expose_secret()
    );
    assert_eq!(fetcher.call_count(), 2);
}

#[test]
fn gcp_iam_credential_provider_fetches_through_its_cache() {
    let clock = FakeClock::new();
    let fetcher = CountingGcpFetcher::new();
    let provider = GcpIamCredentialProvider {
        service_account: SecretString::from("redis-sa".to_owned()),
        ttl: Duration::from_mins(55),
        cache: Arc::new(GcpTokenCache::default()),
    };
    let token = provider
        .cache
        .get_or_fetch(
            provider.service_account.expose_secret(),
            provider.ttl,
            &clock,
            &fetcher,
        )
        .unwrap();
    assert_eq!(token.expose_secret(), "redis-sa-token-1");
}

struct StaticAwsCredentials;

impl AwsCredentialsResolver for StaticAwsCredentials {
    fn resolve(&self) -> Result<Credentials, super::aws::ElastiCacheSigningError> {
        Ok(Credentials::new(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            None,
            None,
            "redis-credentials-tests",
        ))
    }
}

#[test]
fn elasticache_provider_signs_connect_url_for_the_configured_cache() {
    let settings = AwsSettings {
        user_name: "app".to_owned(),
        cache_name: "cache.example".to_owned(),
        region: "eu-west-1".to_owned(),
        serverless: false,
        token_ttl_seconds: 900,
    };
    let provider = ElastiCacheIamProvider { settings };
    let (user_name, signed) = provider
        .get_credentials(&StaticAwsCredentials, SystemTime::UNIX_EPOCH)
        .unwrap();
    let signed_url = signed.expose_secret();
    assert_eq!(user_name, "app");
    assert!(signed_url.starts_with("cache.example/?"));
    assert!(signed_url.contains("Action=connect"));
    assert!(signed_url.contains("User=app"));
    assert!(signed_url.contains("X-Amz-Signature="));
    assert!(signed_url.contains("X-Amz-Expires=900"));
    assert!(!signed_url.contains("ResourceType"));
}

#[test]
fn elasticache_provider_matches_botocore_sigv4_query_auth_known_answer() {
    // Expected value from botocore's SigV4QueryAuth(creds, "elasticache", "eu-west-1", expires=900)
    // signing the same request at the Unix epoch, for the static example credentials below.
    let expected = "cache.example/?Action=connect&User=app\
        &X-Amz-Algorithm=AWS4-HMAC-SHA256\
        &X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F19700101%2Feu-west-1%2Felasticache%2Faws4_request\
        &X-Amz-Date=19700101T000000Z\
        &X-Amz-Expires=900\
        &X-Amz-SignedHeaders=host\
        &X-Amz-Signature=e1e4ce13c45cfa056cbbb7bf8235fe64d59d0f2b10176bea67739664779bcfd4";
    let settings = AwsSettings {
        user_name: "app".to_owned(),
        cache_name: "cache.example".to_owned(),
        region: "eu-west-1".to_owned(),
        serverless: false,
        token_ttl_seconds: 900,
    };
    let provider = ElastiCacheIamProvider { settings };
    let (_user_name, signed) = provider
        .get_credentials(&StaticAwsCredentials, SystemTime::UNIX_EPOCH)
        .unwrap();
    assert_eq!(signed.expose_secret(), expected);
}

#[test]
fn elasticache_provider_marks_serverless_caches_in_the_signed_url() {
    let settings = AwsSettings {
        user_name: "app".to_owned(),
        cache_name: "cache".to_owned(),
        region: "eu-west-1".to_owned(),
        serverless: true,
        token_ttl_seconds: 900,
    };
    let provider = ElastiCacheIamProvider { settings };
    let (_user_name, signed) = provider
        .get_credentials(&StaticAwsCredentials, SystemTime::UNIX_EPOCH)
        .unwrap();
    assert!(
        signed
            .expose_secret()
            .contains("ResourceType=ServerlessCache")
    );
}

#[derive(Debug)]
struct FixedFetcherLockPoisoned;

impl fmt::Display for FixedFetcherLockPoisoned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the fixed azure token fetcher's lock was poisoned")
    }
}

impl StdError for FixedFetcherLockPoisoned {}

struct FixedAzureTokenFetcher {
    token: String,
    scopes_seen: Mutex<Vec<String>>,
}

impl FixedAzureTokenFetcher {
    fn call_count(&self) -> usize {
        self.scopes_seen.lock().unwrap().len()
    }
}

impl AzureTokenFetcher for FixedAzureTokenFetcher {
    fn fetch_token(
        &self,
        _credential: AzureCredential,
        scope: &str,
    ) -> Result<AzureToken, AzureTokenError> {
        let mut scopes_seen =
            self.scopes_seen
                .lock()
                .map_err(|_poisoned| AzureTokenError::Fetch {
                    source: Box::new(FixedFetcherLockPoisoned),
                })?;
        scopes_seen.push(scope.to_owned());
        drop(scopes_seen);
        Ok(AzureToken {
            token: SecretString::from(self.token.clone()),
        })
    }
}

#[test]
fn azure_ad_provider_returns_username_and_token_from_its_fetcher() {
    let fetcher = Arc::new(FixedAzureTokenFetcher {
        token: "access-token".to_owned(),
        scopes_seen: Mutex::new(Vec::new()),
    });
    let cloned_fetcher = Arc::clone(&fetcher);
    let fetcher_handle: Arc<dyn AzureTokenFetcher> = cloned_fetcher;
    let provider = AzureAdCredentialProvider {
        credential: AzureCredential::ManagedIdentity,
        fetcher: fetcher_handle,
        username: Some("app".to_owned()),
    };
    let (username, token) = provider.get_credentials().unwrap();
    assert_eq!(username, Some("app".to_owned()));
    assert_eq!(token.expose_secret(), "access-token");
    assert_eq!(
        fetcher.scopes_seen.lock().unwrap().as_slice(),
        [AZURE_REDIS_SCOPE]
    );
}

#[test]
fn azure_ad_provider_reuses_the_same_fetcher_across_calls() {
    let fetcher = Arc::new(FixedAzureTokenFetcher {
        token: "access-token".to_owned(),
        scopes_seen: Mutex::new(Vec::new()),
    });
    let cloned_fetcher = Arc::clone(&fetcher);
    let fetcher_handle: Arc<dyn AzureTokenFetcher> = cloned_fetcher;
    let provider = AzureAdCredentialProvider {
        credential: AzureCredential::Default,
        fetcher: fetcher_handle,
        username: None,
    };
    provider.get_credentials().unwrap();
    provider.get_credentials().unwrap();
    assert_eq!(fetcher.call_count(), 2);
    assert_eq!(Arc::strong_count(&fetcher), 2);
}
