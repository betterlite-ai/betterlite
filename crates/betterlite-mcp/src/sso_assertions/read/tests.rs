#![cfg(test)]

use std::convert::Infallible;
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_db::sso_assertion_rows::{
    AssertionRow, AssertionRows, MemoryAssertionRows, MemoryFailures, MemoryRowsError,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, OsNonces, StoredCipher, StoredValue,
};
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::{SsoAssertionStore as _, fetch_uncached};
use crate::credential_contracts::byok_cache::SystemClock;
use crate::sso_assertions::assertion::SsoIdentityAssertion;
use crate::sso_assertions::cache::{CacheState, SsoAssertionCache};
use crate::sso_assertions::store::DbSsoAssertionStore;

const SALT_KEY: &str = "b10064-salt-key";
const EXPIRES_AT: i64 = 1_893_456_000;
const PAST: i64 = 1_600_000_000;

fn cipher_with(key: Option<&str>) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: key.map(|text| SecretString::from(text.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn empty_cache() -> Arc<SsoAssertionCache> {
    Arc::new(SsoAssertionCache {
        ttl: Duration::from_secs(60),
        max_size: 8,
        clock: Box::new(SystemClock),
        state: Mutex::new(CacheState::default()),
    })
}

fn memory_rows(failures: MemoryFailures) -> MemoryAssertionRows {
    MemoryAssertionRows {
        rows: Mutex::new(Vec::new()),
        id_jag: false,
        failures,
        reads: AtomicUsize::new(0),
        writes: AtomicUsize::new(0),
    }
}

fn store_with(
    rows: Option<MemoryAssertionRows>,
    cache: Arc<SsoAssertionCache>,
) -> DbSsoAssertionStore<MemoryAssertionRows> {
    DbSsoAssertionStore {
        rows,
        cipher: cipher_with(Some(SALT_KEY)),
        cache,
        config_declares_id_jag: false,
    }
}

fn assertion(token: &str, refresh_token: Option<&str>) -> SsoIdentityAssertion {
    SsoIdentityAssertion {
        id_token: SecretString::from(token.to_owned()),
        refresh_token: refresh_token.map(|text| SecretString::from(text.to_owned())),
        issuer: Some("https://idp.test".to_owned()),
        expires_at: DateTime::from_timestamp(EXPIRES_AT, 0),
    }
}

fn cached_token(cache: &SsoAssertionCache, user_id: &str) -> Option<String> {
    cache
        .get(user_id)
        .map(|found| found.id_token.expose_secret().to_owned())
}

fn sealed_payload(payload: &str) -> String {
    let sealed = cipher_with(Some(SALT_KEY))
        .encrypt_value(
            StoredValue::<Infallible>::Text(SecretString::from(payload.to_owned())),
            None,
        )
        .unwrap();
    match sealed {
        StoredValue::Text(text) => text.expose_secret().to_owned(),
        StoredValue::Other(never) => match never {},
    }
}

fn seed_row(rows: &MemoryAssertionRows, user_id: &str, payload: &str) {
    rows.rows
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(AssertionRow {
            user_id: user_id.to_owned(),
            assertion_b64: sealed_payload(payload),
        });
}

fn reads(rows: &MemoryAssertionRows) -> usize {
    rows.reads.load(Ordering::SeqCst)
}

// every find invalidates the cache first, the login that lands during a fetch's row read
struct InvalidatingRows {
    rows: MemoryAssertionRows,
    cache: Arc<SsoAssertionCache>,
}

impl AssertionRows for InvalidatingRows {
    type Error = MemoryRowsError;

    async fn find(&self, user_id: &str) -> Result<Option<AssertionRow>, MemoryRowsError> {
        self.cache.invalidate(user_id);
        self.rows.find(user_id).await
    }

    async fn find_all(&self) -> Result<Vec<AssertionRow>, MemoryRowsError> {
        self.rows.find_all().await
    }

    async fn upsert(&self, user_id: &str, assertion_b64: &str) -> Result<(), MemoryRowsError> {
        self.rows.upsert(user_id, assertion_b64).await
    }

    async fn update(&self, user_id: &str, assertion_b64: &str) -> Result<(), MemoryRowsError> {
        self.rows.update(user_id, assertion_b64).await
    }

    async fn id_jag_server_exists(&self) -> Result<bool, MemoryRowsError> {
        self.rows.id_jag_server_exists().await
    }
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

impl Logs {
    fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }
}

impl<Current> Layer<Current> for Logs
where
    Current: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Current>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

#[tokio::test]
async fn fetch_returns_a_cache_hit_without_reading_the_row() {
    let cache = empty_cache();
    cache.set_if_unchanged("alice", &assertion("cached-token", None), cache.epoch());
    let store = store_with(
        Some(memory_rows(MemoryFailures::default())),
        Arc::clone(&cache),
    );
    let found = store.fetch("alice").await.unwrap().unwrap();
    assert_eq!(found.id_token.expose_secret(), "cached-token");
    assert_eq!(reads(store.rows.as_ref().unwrap()), 0);
}

#[tokio::test]
async fn fetch_caches_a_found_row_and_skips_the_row_on_the_second_call() {
    let cache = empty_cache();
    let store = store_with(
        Some(memory_rows(MemoryFailures::default())),
        Arc::clone(&cache),
    );
    store
        .persist("alice", &assertion("live-token", Some("refresh-token")))
        .await
        .unwrap();
    let first = store.fetch("alice").await.unwrap().unwrap();
    assert_eq!(first.id_token.expose_secret(), "live-token");
    assert_eq!(
        first
            .refresh_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("refresh-token")
    );
    assert_eq!(reads(store.rows.as_ref().unwrap()), 1);
    let second = store.fetch("alice").await.unwrap().unwrap();
    assert_eq!(second.id_token.expose_secret(), "live-token");
    assert_eq!(reads(store.rows.as_ref().unwrap()), 1);
    assert_eq!(cached_token(&cache, "alice").as_deref(), Some("live-token"));
}

#[tokio::test]
async fn fetch_reads_the_row_again_after_an_absent_result() {
    let cache = empty_cache();
    let store = store_with(
        Some(memory_rows(MemoryFailures::default())),
        Arc::clone(&cache),
    );
    assert!(store.fetch("alice").await.unwrap().is_none());
    assert!(store.fetch("alice").await.unwrap().is_none());
    assert_eq!(reads(store.rows.as_ref().unwrap()), 2);
    assert!(cache.get("alice").is_none());
}

#[tokio::test]
async fn fetch_returns_an_expired_assertion_unjudged() {
    let cache = empty_cache();
    let store = store_with(
        Some(memory_rows(MemoryFailures::default())),
        Arc::clone(&cache),
    );
    let expired = SsoIdentityAssertion {
        id_token: SecretString::from("expired-token".to_owned()),
        refresh_token: Some(SecretString::from("expired-refresh".to_owned())),
        issuer: None,
        expires_at: DateTime::from_timestamp(PAST, 0),
    };
    store.persist("alice", &expired).await.unwrap();
    let found = store.fetch("alice").await.unwrap().unwrap();
    assert_eq!(found.id_token.expose_secret(), "expired-token");
    assert_eq!(found.expires_at, DateTime::from_timestamp(PAST, 0));
    assert_eq!(
        cached_token(&cache, "alice").as_deref(),
        Some("expired-token")
    );
}

#[tokio::test]
async fn an_invalidate_during_the_row_read_stops_the_fetch_from_caching() {
    let cache = empty_cache();
    let store = DbSsoAssertionStore::<InvalidatingRows> {
        rows: Some(InvalidatingRows {
            rows: memory_rows(MemoryFailures::default()),
            cache: Arc::clone(&cache),
        }),
        cipher: cipher_with(Some(SALT_KEY)),
        cache: Arc::clone(&cache),
        config_declares_id_jag: false,
    };
    store
        .persist("alice", &assertion("old-token", None))
        .await
        .unwrap();
    let found = store.fetch("alice").await.unwrap().unwrap();
    assert_eq!(found.id_token.expose_secret(), "old-token");
    assert!(cache.get("alice").is_none());
    assert_eq!(cache.epoch(), 2);
    assert_eq!(reads(&store.rows.as_ref().unwrap().rows), 1);
}

#[tokio::test]
async fn fetch_uncached_reads_the_row_every_time_and_leaves_the_cache_as_it_was() {
    let rows = memory_rows(MemoryFailures::default());
    seed_row(&rows, "alice", r#"{"id_token":"row-token"}"#);
    let cache = empty_cache();
    cache.set_if_unchanged("alice", &assertion("cached-token", None), cache.epoch());
    let store = store_with(Some(rows), Arc::clone(&cache));
    let first = fetch_uncached(&store, "alice").await.unwrap().unwrap();
    assert_eq!(first.id_token.expose_secret(), "row-token");
    assert_eq!(
        cached_token(&cache, "alice").as_deref(),
        Some("cached-token")
    );
    let second = fetch_uncached(&store, "alice").await.unwrap().unwrap();
    assert_eq!(second.id_token.expose_secret(), "row-token");
    assert_eq!(reads(store.rows.as_ref().unwrap()), 2);
    assert_eq!(
        cached_token(&cache, "alice").as_deref(),
        Some("cached-token")
    );
}

#[tokio::test]
async fn fetch_reports_a_row_read_failure_as_an_unavailable_store() {
    let store = store_with(
        Some(memory_rows(MemoryFailures {
            find: Some(0),
            find_all: None,
            upsert: None,
            update: None,
            id_jag_server_exists: None,
        })),
        empty_cache(),
    );
    let error = store.fetch("alice").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "the SSO identity assertion store is unavailable: the in-memory assertion rows failed on Find"
    );
    assert_eq!(
        error.cause.to_string(),
        "the in-memory assertion rows failed on Find"
    );
    assert_eq!(reads(store.rows.as_ref().unwrap()), 0);
}

#[tokio::test]
async fn fetch_uncached_reports_a_row_read_failure_as_an_unavailable_store() {
    let store = store_with(
        Some(memory_rows(MemoryFailures {
            find: Some(0),
            find_all: None,
            upsert: None,
            update: None,
            id_jag_server_exists: None,
        })),
        empty_cache(),
    );
    let error = fetch_uncached(&store, "alice").await.unwrap_err();
    assert_eq!(
        error.cause.to_string(),
        "the in-memory assertion rows failed on Find"
    );
}

#[tokio::test]
async fn no_database_reads_as_absent_from_both_methods() {
    let cache = empty_cache();
    let store = store_with(None, Arc::clone(&cache));
    assert!(store.fetch("alice").await.unwrap().is_none());
    assert!(fetch_uncached(&store, "alice").await.unwrap().is_none());
    assert!(cache.get("alice").is_none());
}

#[tokio::test]
async fn a_row_that_will_not_decrypt_reads_as_absent() {
    let rows = memory_rows(MemoryFailures::default());
    rows.rows
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(AssertionRow {
            user_id: "alice".to_owned(),
            assertion_b64: "not-a-ciphertext".to_owned(),
        });
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    let cache = empty_cache();
    let store = store_with(Some(rows), Arc::clone(&cache));
    assert!(fetch_uncached(&store, "alice").await.unwrap().is_none());
    assert!(store.fetch("alice").await.unwrap().is_none());
    assert_eq!(reads(store.rows.as_ref().unwrap()), 2);
    assert!(cache.get("alice").is_none());
    assert!(logs.messages().is_empty());
}

#[tokio::test]
async fn an_unparsable_payload_logs_the_warning_and_reads_as_absent() {
    let rows = memory_rows(MemoryFailures::default());
    seed_row(&rows, "alice", r#"{"refresh_token":"orphan-refresh"}"#);
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    let cache = empty_cache();
    let store = store_with(Some(rows), Arc::clone(&cache));
    assert!(fetch_uncached(&store, "alice").await.unwrap().is_none());
    assert!(store.fetch("alice").await.unwrap().is_none());
    assert_eq!(
        logs.messages(),
        vec![
            "Stored SSO identity assertion for user_id=alice could not be parsed; treating as absent."
                .to_owned(),
            "Stored SSO identity assertion for user_id=alice could not be parsed; treating as absent."
                .to_owned(),
        ]
    );
    assert!(cache.get("alice").is_none());
}

#[tokio::test]
async fn a_stored_payload_with_only_some_fields_reads_back() {
    let rows = memory_rows(MemoryFailures::default());
    seed_row(&rows, "alice", r#"{"id_token":"only-token"}"#);
    seed_row(
        &rows,
        "bravo",
        r#"{"id_token":"empty-claims-token","refresh_token":"","issuer":""}"#,
    );
    let store = store_with(Some(rows), empty_cache());
    let only = fetch_uncached(&store, "alice").await.unwrap().unwrap();
    assert_eq!(only.id_token.expose_secret(), "only-token");
    assert!(only.refresh_token.is_none());
    assert!(only.issuer.is_none());
    assert!(only.expires_at.is_none());
    let empty = fetch_uncached(&store, "bravo").await.unwrap().unwrap();
    assert_eq!(empty.id_token.expose_secret(), "empty-claims-token");
    assert!(empty.refresh_token.is_none());
    assert_eq!(empty.issuer.as_deref(), Some(""));
    assert!(empty.expires_at.is_none());
}

#[tokio::test]
async fn a_naive_expires_at_reads_as_utc() {
    let rows = memory_rows(MemoryFailures::default());
    seed_row(
        &rows,
        "alice",
        r#"{"id_token":"naive-token","issuer":"https://idp.test","expires_at":"2031-02-03T04:05:06"}"#,
    );
    let store = store_with(Some(rows), empty_cache());
    let found = fetch_uncached(&store, "alice").await.unwrap().unwrap();
    assert_eq!(found.id_token.expose_secret(), "naive-token");
    assert_eq!(found.issuer.as_deref(), Some("https://idp.test"));
    assert_eq!(
        found.expires_at,
        Some(
            DateTime::parse_from_rfc3339("2031-02-03T04:05:06+00:00")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
}

#[tokio::test]
async fn no_token_text_reaches_logs_or_debug_output() {
    let rows = memory_rows(MemoryFailures::default());
    seed_row(
        &rows,
        "alice",
        r#"{"id_token":"sk-live-id-token-9f31","refresh_token":"refresh-token-9f31","expires_at":"not-a-date"}"#,
    );
    seed_row(
        &rows,
        "bravo",
        r#"{"id_token":"sk-valid-id-token-9f31","refresh_token":"valid-refresh-token-9f31"}"#,
    );
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    let store = store_with(Some(rows), empty_cache());
    assert!(fetch_uncached(&store, "alice").await.unwrap().is_none());
    let valid = store.fetch("bravo").await.unwrap().unwrap();
    assert!(!format!("{valid:?}").contains("sk-valid-id-token-9f31"));
    assert!(!format!("{valid:?}").contains("valid-refresh-token-9f31"));
    let messages = logs.messages().join(" ");
    assert!(!messages.contains("sk-live-id-token-9f31"));
    assert!(!messages.contains("refresh-token-9f31"));
    let failing = store_with(
        Some(memory_rows(MemoryFailures {
            find: Some(0),
            find_all: None,
            upsert: None,
            update: None,
            id_jag_server_exists: None,
        })),
        empty_cache(),
    );
    let error = failing.fetch("alice").await.unwrap_err();
    assert!(!error.to_string().contains("sk-live-id-token-9f31"));
    assert!(!format!("{error:?}").contains("refresh-token-9f31"));
}
