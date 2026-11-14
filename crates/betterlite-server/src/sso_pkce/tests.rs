#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_cache::{AuthCache, WriteOptions, gateway_cache};
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_config::sso_pkce::PkceSettings;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::NamedTempFile;
use tokio::net::TcpListener;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use super::token_exchange::{TokenExchangeParameters, token_exchange_parameters};
use super::verifier_cache::{
    CacheRead, PkceCacheEntry, PkceCacheKind, PkceCaches, VerifierCache, json_from_cached,
    stored_entry,
};
use super::{
    CODE_CHALLENGE_METHOD, GeneratedPkce, GenericSsoRedirectParams, OsEntropy, PkceCacheShape,
    PkceError, VerifierEntropy, generic_sso_redirect_params,
};

const VERIFIER_FOR_SEQUENCE: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
const CHALLENGE_FOR_SEQUENCE: &str = "6oZqdX5MOLq_qBJ8vppAnT4fk6AP8UiP9zX8-Rev_9A";
const STATE_FOR_SEQUENCE: &str = "000102030405460788090a0b0c0d0e0f";

fn settings(use_pkce: bool) -> PkceSettings {
    PkceSettings {
        use_pkce,
        strict_cache_miss: false,
        generic_client_state: None,
    }
}

fn strict(base: PkceSettings) -> PkceSettings {
    PkceSettings {
        strict_cache_miss: true,
        ..base
    }
}

struct SequenceEntropy;

impl VerifierEntropy for SequenceEntropy {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), getrandom::Error> {
        for (byte, value) in bytes.iter_mut().zip(0_u8..) {
            *byte = value;
        }
        Ok(())
    }
}

struct BrokenEntropy;

impl VerifierEntropy for BrokenEntropy {
    fn fill(&self, _bytes: &mut [u8]) -> Result<(), getrandom::Error> {
        Err(getrandom::Error::UNSUPPORTED)
    }
}

struct FakeCache {
    kind: PkceCacheKind,
    entry: Option<PkceCacheEntry>,
    fails: bool,
    reads: AtomicUsize,
    deletes: Mutex<Vec<String>>,
}

impl FakeCache {
    fn returning(kind: PkceCacheKind, entry: Option<PkceCacheEntry>) -> Self {
        Self {
            kind,
            entry,
            fails: false,
            reads: AtomicUsize::new(0),
            deletes: Mutex::new(Vec::new()),
        }
    }

    fn failing(kind: PkceCacheKind) -> Self {
        Self {
            kind,
            entry: None,
            fails: true,
            reads: AtomicUsize::new(0),
            deletes: Mutex::new(Vec::new()),
        }
    }

    fn deletes(&self) -> Vec<String> {
        self.deletes.lock().unwrap().clone()
    }
}

#[async_trait]
impl VerifierCache for FakeCache {
    fn kind(&self) -> PkceCacheKind {
        self.kind
    }

    async fn read(&self, _cache_key: &str) -> CacheRead {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.fails {
            return CacheRead::Miss;
        }
        self.entry.clone().map_or(CacheRead::Miss, CacheRead::Found)
    }

    async fn delete(&self, cache_key: &str) {
        self.deletes.lock().unwrap().push(cache_key.to_owned());
    }
}

struct SilentHooks;

impl ServiceHooks for SilentHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn StdError + Send + Sync)) {}
}

struct AllowGate;

impl BreakerGate for AllowGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(1))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn StdError + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn StdError + Send + Sync)) {}
}

async fn unreachable_cache() -> RedisCache {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let overrides: RedisOverrides = serde_json::from_str(&format!(
        r#"{{"host": "127.0.0.1", "port": {port}, "socket_timeout": 0.05, "socket_connect_timeout": 0.05}}"#
    ))
    .unwrap();
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(SilentHooks),
        gate: Arc::new(AllowGate),
    };
    connect(options, &BTreeMap::<String, String>::new())
        .await
        .unwrap()
}

fn clock() -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(|| 1_000.0_f64)
}

fn memory() -> Arc<LocalMemoryOps> {
    Arc::new(LocalMemoryOps::new(1024, clock()).unwrap())
}

fn local_cache() -> AuthCache<DualCache<LocalMemoryOps>> {
    gateway_cache(
        memory(),
        memory(),
        None,
        DualCacheSettings {
            default_in_memory_ttl: None,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: 1000,
            redis_delete_chunk_size: 1000,
        },
        45.0_f64,
    )
}

fn object(field: &str, value: Json) -> Json {
    Json::Object(IndexMap::from([(field.to_owned(), value)]))
}

fn capture_logs(level: tracing::Level) -> (NamedTempFile, DefaultGuard) {
    let log = NamedTempFile::new().unwrap();
    let subscriber = fmt()
        .without_time()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    (log, set_default(subscriber))
}

fn written(log: &NamedTempFile) -> String {
    fs::read_to_string(log.path()).unwrap()
}

#[test]
fn generate_pkce_params_matches_the_rfc7636_vector() {
    let GeneratedPkce {
        verifier,
        challenge,
    } = super::generate_pkce_params(&SequenceEntropy).unwrap();
    assert_eq!(verifier.expose_secret(), VERIFIER_FOR_SEQUENCE);
    assert_eq!(challenge, CHALLENGE_FOR_SEQUENCE);
    assert_eq!(verifier.expose_secret().len(), 43);
    assert_eq!(challenge.len(), 43);
    assert!(!verifier.expose_secret().contains('='));
    assert!(!challenge.contains('='));
}

#[test]
fn generate_pkce_params_reports_a_broken_entropy_source() {
    let error = super::generate_pkce_params(&BrokenEntropy).unwrap_err();
    assert!(matches!(&error, PkceError::Entropy(_)));
    let response = error.response();
    assert_eq!(response.status, 500);
    assert_eq!(response.error_type, ProxyErrorTypes::InternalServerError);
    assert_eq!(response.param, None);
    assert_eq!(response.code, "500");
}

#[test]
fn a_supplied_cli_state_wins_over_the_configured_state() {
    let configured = PkceSettings {
        generic_client_state: Some("configured".to_owned()),
        ..settings(false)
    };
    let redirect = generic_sso_redirect_params(Some("cli"), &configured, &SequenceEntropy).unwrap();
    assert_eq!(redirect.params.state, "cli");
    assert!(redirect.code_verifier.is_none());
}

#[test]
fn an_empty_cli_state_falls_through_to_the_configured_state() {
    let configured = PkceSettings {
        generic_client_state: Some("configured".to_owned()),
        ..settings(false)
    };
    let redirect = generic_sso_redirect_params(Some(""), &configured, &SequenceEntropy).unwrap();
    assert_eq!(redirect.params.state, "configured");
}

#[test]
fn a_missing_state_becomes_a_uuid_hex_value() {
    let redirect = generic_sso_redirect_params(None, &settings(false), &OsEntropy).unwrap();
    assert_eq!(redirect.params.state.len(), 32);
    assert!(!redirect.params.state.contains('-'));
    assert!(
        redirect
            .params
            .state
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
    );
    let parsed = uuid::Uuid::parse_str(&redirect.params.state).unwrap();
    assert_eq!(parsed.simple().to_string(), redirect.params.state);
    let other = generic_sso_redirect_params(None, &settings(false), &OsEntropy)
        .unwrap()
        .params
        .state;
    assert_ne!(redirect.params.state, other);
}

#[test]
fn the_generated_state_comes_from_the_injected_entropy() {
    let redirect = generic_sso_redirect_params(None, &settings(false), &SequenceEntropy).unwrap();
    assert_eq!(redirect.params.state, STATE_FOR_SEQUENCE);
}

#[test]
fn pkce_parameters_are_absent_when_pkce_is_disabled() {
    let redirect =
        generic_sso_redirect_params(Some("state"), &settings(false), &SequenceEntropy).unwrap();
    assert_eq!(redirect.params.pkce, None);
    assert!(redirect.code_verifier.is_none());
    assert_eq!(
        serde_json::to_string(&redirect.params).unwrap(),
        r#"{"state":"state"}"#
    );
}

#[test]
fn pkce_parameters_carry_the_s256_challenge_and_the_verifier() {
    let redirect =
        generic_sso_redirect_params(Some("state"), &settings(true), &SequenceEntropy).unwrap();
    let pkce = redirect.params.pkce.as_ref().unwrap();
    assert_eq!(pkce.code_challenge, CHALLENGE_FOR_SEQUENCE);
    assert_eq!(pkce.code_challenge_method, CODE_CHALLENGE_METHOD);
    assert_eq!(
        redirect.code_verifier.unwrap().expose_secret(),
        VERIFIER_FOR_SEQUENCE
    );
    assert_eq!(
        serde_json::to_string(&redirect.params).unwrap(),
        format!(
            r#"{{"state":"state","code_challenge":"{CHALLENGE_FOR_SEQUENCE}","code_challenge_method":"S256"}}"#
        )
    );
}

#[test]
fn the_os_entropy_source_fills_a_verifier_of_the_expected_shape() {
    let generated = super::generate_pkce_params(&OsEntropy).unwrap();
    assert_eq!(generated.verifier.expose_secret().len(), 43);
    assert_eq!(generated.challenge.len(), 43);
    let second = super::generate_pkce_params(&OsEntropy).unwrap();
    assert_ne!(
        generated.verifier.expose_secret(),
        second.verifier.expose_secret()
    );
}

#[rstest]
#[case::object_with_a_verifier(object("code_verifier", Json::Text("v".to_owned())), "verifier")]
#[case::object_with_an_empty_verifier(object("code_verifier", Json::Text(String::new())), "empty")]
#[case::object_with_a_null_verifier(object("code_verifier", Json::Null), "empty")]
#[case::object_without_the_field(Json::Object(IndexMap::new()), "unexpected_object")]
#[case::object_with_a_number_verifier(
    object("code_verifier", Json::Number(serde_json::Number::from(1_i64))),
    "unexpected_object"
)]
#[case::legacy_text(Json::Text("legacy".to_owned()), "legacy")]
#[case::empty_legacy_text(Json::Text(String::new()), "empty")]
#[case::boolean(Json::Bool(true), "unexpected_bool")]
#[case::number(Json::Number(serde_json::Number::from(2_i64)), "unexpected_number")]
#[case::list(Json::List(Vec::new()), "unexpected_list")]
#[case::null(Json::Null, "missing")]
fn stored_entry_classifies_cached_values(#[case] value: Json, #[case] expected: &str) {
    let actual = stored_entry(&value).map(|entry| match entry {
        PkceCacheEntry::Verifier(_) => "verifier",
        PkceCacheEntry::LegacyVerifier(_) => "legacy",
        PkceCacheEntry::EmptyVerifier => "empty",
        PkceCacheEntry::UnexpectedShape(PkceCacheShape::Object) => "unexpected_object",
        PkceCacheEntry::UnexpectedShape(PkceCacheShape::Boolean) => "unexpected_bool",
        PkceCacheEntry::UnexpectedShape(PkceCacheShape::Number) => "unexpected_number",
        PkceCacheEntry::UnexpectedShape(PkceCacheShape::List) => "unexpected_list",
        PkceCacheEntry::UnexpectedShape(PkceCacheShape::Null) => "unexpected_null",
    });
    assert_eq!(actual.unwrap_or("missing"), expected);
}

#[rstest]
#[case::null(CachedValue::Null, Json::Null)]
#[case::boolean(CachedValue::Boolean(true), Json::Bool(true))]
#[case::number(
    CachedValue::Number(serde_json::Number::from(7_i64)),
    Json::Number(serde_json::Number::from(7_i64))
)]
#[case::text(CachedValue::Text("text".to_owned()), Json::Text("text".to_owned()))]
#[case::array(
    CachedValue::Array(vec![CachedValue::Number(serde_json::Number::from(1_i64))]),
    Json::List(vec![Json::Number(serde_json::Number::from(1_i64))])
)]
#[case::object(
    CachedValue::Object(BTreeMap::from([(
        "code_verifier".to_owned(),
        CachedValue::Text("v".to_owned())
    )])),
    object("code_verifier", Json::Text("v".to_owned()))
)]
fn json_from_cached_preserves_each_cached_value(
    #[case] cached: CachedValue,
    #[case] expected: Json,
) {
    assert_eq!(json_from_cached(cached), expected);
}

fn verifier_of(parameters: &TokenExchangeParameters) -> Option<String> {
    parameters
        .verifier
        .as_ref()
        .map(|verifier| verifier.code_verifier.expose_secret().to_owned())
}

#[tokio::test]
async fn a_cached_object_verifier_is_returned_with_its_cache_key() {
    let cache = FakeCache::returning(
        PkceCacheKind::UserCache,
        Some(PkceCacheEntry::Verifier(SecretString::from(
            "verify-me".to_owned(),
        ))),
    );
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let parameters = token_exchange_parameters(Some("state"), true, &settings(true), &caches)
        .await
        .unwrap();
    assert!(parameters.include_client_id);
    assert_eq!(verifier_of(&parameters).as_deref(), Some("verify-me"));
    assert_eq!(
        parameters.verifier.unwrap().cache_key,
        "pkce_verifier:state"
    );
    assert!(cache.deletes().is_empty());
}

#[tokio::test]
async fn a_legacy_string_verifier_stays_usable_and_warns() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let cache = FakeCache::returning(
        PkceCacheKind::UserCache,
        Some(PkceCacheEntry::LegacyVerifier(SecretString::from(
            "legacy".to_owned(),
        ))),
    );
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let parameters = token_exchange_parameters(Some("state"), false, &settings(true), &caches)
        .await
        .unwrap();
    assert_eq!(verifier_of(&parameters).as_deref(), Some("legacy"));
    assert!(cache.deletes().is_empty());
    assert!(written(&log).contains("legacy plain-string format"));
}

#[tokio::test]
async fn a_missing_state_only_warns_even_in_strict_mode() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let cache = FakeCache::returning(PkceCacheKind::UserCache, None);
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let parameters = token_exchange_parameters(None, true, &strict(settings(true)), &caches)
        .await
        .unwrap();
    assert!(parameters.verifier.is_none());
    assert_eq!(cache.reads.load(Ordering::SeqCst), 0);
    assert!(written(&log).contains("carried no state parameter"));
}

#[tokio::test]
async fn an_empty_callback_state_counts_as_missing() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let cache = FakeCache::returning(PkceCacheKind::UserCache, None);
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let parameters = token_exchange_parameters(Some(""), true, &strict(settings(true)), &caches)
        .await
        .unwrap();
    assert!(parameters.verifier.is_none());
    assert_eq!(cache.reads.load(Ordering::SeqCst), 0);
    assert!(written(&log).contains("carried no state parameter"));
}

#[tokio::test]
async fn pkce_disabled_without_a_state_does_not_warn() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let cache = FakeCache::returning(PkceCacheKind::UserCache, None);
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let parameters = token_exchange_parameters(None, true, &strict(settings(false)), &caches)
        .await
        .unwrap();
    assert!(parameters.verifier.is_none());
    assert!(written(&log).is_empty());
}

#[tokio::test]
async fn pkce_disabled_never_touches_the_cache() {
    let cache = FakeCache::returning(
        PkceCacheKind::UserCache,
        Some(PkceCacheEntry::EmptyVerifier),
    );
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let parameters =
        token_exchange_parameters(Some("state"), true, &strict(settings(false)), &caches)
            .await
            .unwrap();
    assert!(parameters.verifier.is_none());
    assert_eq!(cache.reads.load(Ordering::SeqCst), 0);
    assert!(cache.deletes().is_empty());
}

#[tokio::test]
async fn a_strict_cache_miss_returns_the_pkce_cache_miss_error() {
    let (log, _guard) = capture_logs(tracing::Level::ERROR);
    let cache = FakeCache::returning(PkceCacheKind::UserCache, None);
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let error = token_exchange_parameters(Some("state"), true, &strict(settings(true)), &caches)
        .await
        .unwrap_err();
    assert!(matches!(&error, PkceError::MissingVerifier { .. }));
    let response = error.response();
    assert_eq!(response.status, 401);
    assert_eq!(response.error_type, ProxyErrorTypes::AuthError);
    assert_eq!(response.param, Some("PKCE_CACHE_MISS"));
    assert_eq!(response.code, "401");
    assert!(cache.deletes().is_empty());
    assert!(written(&log).contains("no verifier was found in cache"));
}

#[tokio::test]
async fn a_permissive_cache_miss_continues_without_a_verifier() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let cache = FakeCache::returning(PkceCacheKind::UserCache, None);
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let parameters = token_exchange_parameters(Some("state"), false, &settings(true), &caches)
        .await
        .unwrap();
    assert!(parameters.verifier.is_none());
    assert!(!parameters.include_client_id);
    assert!(cache.deletes().is_empty());
    assert!(written(&log).contains("set PKCE_STRICT_CACHE_MISS=true"));
}

#[tokio::test]
async fn an_empty_verifier_field_is_deleted_and_fails_in_strict_mode() {
    let cache = FakeCache::returning(
        PkceCacheKind::UserCache,
        Some(PkceCacheEntry::EmptyVerifier),
    );
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let error = token_exchange_parameters(Some("state"), true, &strict(settings(true)), &caches)
        .await
        .unwrap_err();
    assert!(matches!(&error, PkceError::EmptyVerifier { .. }));
    assert_eq!(error.response().status, 401);
    assert_eq!(error.response().param, Some("PKCE_CACHE_MISS"));
    assert_eq!(cache.deletes(), vec!["pkce_verifier:state".to_owned()]);
}

#[tokio::test]
async fn an_empty_verifier_field_is_deleted_and_warns_when_permissive() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let cache = FakeCache::returning(
        PkceCacheKind::UserCache,
        Some(PkceCacheEntry::EmptyVerifier),
    );
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let parameters = token_exchange_parameters(Some("state"), true, &settings(true), &caches)
        .await
        .unwrap();
    assert!(parameters.verifier.is_none());
    assert_eq!(cache.deletes(), vec!["pkce_verifier:state".to_owned()]);
    assert!(written(&log).contains("empty or null code_verifier"));
}

#[tokio::test]
async fn a_corrupt_entry_is_deleted_before_the_strict_error() {
    let (log, _guard) = capture_logs(tracing::Level::ERROR);
    let cache = FakeCache::returning(
        PkceCacheKind::UserCache,
        Some(PkceCacheEntry::UnexpectedShape(PkceCacheShape::List)),
    );
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let error = token_exchange_parameters(Some("state"), true, &strict(settings(true)), &caches)
        .await
        .unwrap_err();
    assert!(matches!(&error, PkceError::UnexpectedShape { .. }));
    assert!(error.to_string().contains("(List)"));
    assert_eq!(error.response().code, "401");
    assert_eq!(cache.deletes(), vec!["pkce_verifier:state".to_owned()]);
    assert!(written(&log).contains("unrecognized format"));
}

#[tokio::test]
async fn a_corrupt_entry_is_deleted_and_warns_when_permissive() {
    let cache = FakeCache::returning(
        PkceCacheKind::UserCache,
        Some(PkceCacheEntry::UnexpectedShape(PkceCacheShape::Object)),
    );
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let parameters = token_exchange_parameters(Some("state"), true, &settings(true), &caches)
        .await
        .unwrap();
    assert!(parameters.verifier.is_none());
    assert_eq!(cache.deletes(), vec!["pkce_verifier:state".to_owned()]);
}

#[tokio::test]
async fn the_redis_usage_cache_is_read_when_one_is_configured() {
    let redis = FakeCache::returning(
        PkceCacheKind::RedisUsage,
        Some(PkceCacheEntry::Verifier(SecretString::from(
            "from-redis".to_owned(),
        ))),
    );
    let user = FakeCache::returning(
        PkceCacheKind::UserCache,
        Some(PkceCacheEntry::Verifier(SecretString::from(
            "from-user".to_owned(),
        ))),
    );
    let caches = PkceCaches {
        redis: Some(&redis),
        user: &user,
    };
    let parameters = token_exchange_parameters(Some("state"), true, &settings(true), &caches)
        .await
        .unwrap();
    assert_eq!(verifier_of(&parameters).as_deref(), Some("from-redis"));
    assert_eq!(redis.reads.load(Ordering::SeqCst), 1);
    assert_eq!(user.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_user_cache_is_read_when_no_redis_cache_exists() {
    let user = FakeCache::returning(
        PkceCacheKind::UserCache,
        Some(PkceCacheEntry::Verifier(SecretString::from(
            "from-user".to_owned(),
        ))),
    );
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &user,
    };
    let parameters = token_exchange_parameters(Some("state"), true, &settings(true), &caches)
        .await
        .unwrap();
    assert_eq!(verifier_of(&parameters).as_deref(), Some("from-user"));
    assert_eq!(user.reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_strict_miss_names_the_multi_instance_cause_when_redis_is_configured() {
    let redis = FakeCache::returning(PkceCacheKind::RedisUsage, None);
    let user = FakeCache::returning(PkceCacheKind::UserCache, None);
    let caches = PkceCaches {
        redis: Some(&redis),
        user: &user,
    };
    let error = token_exchange_parameters(Some("state"), true, &strict(settings(true)), &caches)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("different instances"));
}

#[tokio::test]
async fn a_failed_cache_read_is_a_miss() {
    let cache = FakeCache::failing(PkceCacheKind::UserCache);
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let parameters = token_exchange_parameters(Some("state"), true, &settings(true), &caches)
        .await
        .unwrap();
    assert!(parameters.verifier.is_none());
}

#[tokio::test]
async fn a_strict_miss_on_the_auth_cache_names_the_restart_cause() {
    let cache = local_cache();
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let error = token_exchange_parameters(Some("state"), true, &strict(settings(true)), &caches)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Configure Redis"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_redis_usage_cache_reads_as_a_miss() {
    let cache = unreachable_cache().await;
    assert_eq!(cache.kind(), PkceCacheKind::RedisUsage);
    assert!(matches!(
        cache.read("pkce_verifier:state").await,
        CacheRead::Miss
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redis_usage_cache_delete_failure_only_warns() {
    let (log, _guard) = capture_logs(tracing::Level::WARN);
    let cache = unreachable_cache().await;
    cache.delete("pkce_verifier:state").await;
    assert!(written(&log).contains("best-effort cleanup"));
}

#[tokio::test]
async fn a_user_cache_entry_round_trips_through_the_auth_cache() {
    let cache = local_cache();
    cache
        .async_set(
            Some("pkce_verifier:state"),
            &object("code_verifier", Json::Text("stored-verifier".to_owned())),
            WriteOptions::default(),
        )
        .await
        .unwrap();
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    let parameters = token_exchange_parameters(Some("state"), true, &settings(true), &caches)
        .await
        .unwrap();
    let verifier = parameters.verifier.unwrap();
    assert_eq!(verifier.code_verifier.expose_secret(), "stored-verifier");
    assert_eq!(verifier.cache_key, "pkce_verifier:state");
}

#[tokio::test]
async fn deleting_a_verifier_removes_the_user_cache_entry() {
    let cache = local_cache();
    cache
        .async_set(
            Some("pkce_verifier:state"),
            &object("code_verifier", Json::Text("stored-verifier".to_owned())),
            WriteOptions::default(),
        )
        .await
        .unwrap();
    let caches = PkceCaches {
        redis: None::<&FakeCache>,
        user: &cache,
    };
    caches.delete_verifier("pkce_verifier:state").await;
    let parameters = token_exchange_parameters(Some("state"), true, &settings(true), &caches)
        .await
        .unwrap();
    assert!(parameters.verifier.is_none());
}

#[tokio::test]
async fn deleting_a_verifier_uses_the_redis_cache_when_one_is_configured() {
    let redis = FakeCache::returning(PkceCacheKind::RedisUsage, None);
    let user = FakeCache::returning(PkceCacheKind::UserCache, None);
    let caches = PkceCaches {
        redis: Some(&redis),
        user: &user,
    };
    caches.delete_verifier("pkce_verifier:state").await;
    assert_eq!(redis.deletes(), vec!["pkce_verifier:state".to_owned()]);
    assert!(user.deletes().is_empty());
}

#[test]
fn a_serialized_redirect_params_object_omits_the_pkce_fields_when_disabled() {
    let params = GenericSsoRedirectParams {
        state: "state".to_owned(),
        pkce: None,
    };
    assert_eq!(
        serde_json::to_string(&params).unwrap(),
        r#"{"state":"state"}"#
    );
}
