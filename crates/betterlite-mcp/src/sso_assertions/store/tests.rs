#![cfg(test)]

use std::convert::Infallible;
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_db::sso_assertion_rows::{
    ASSERTION_DECRYPT_FIELD, ASSERTION_DECRYPT_POLICY, MemoryAssertionRows, MemoryFailures,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, OsNonces, StoredCipher, StoredValue,
};
use chrono::DateTime;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::{DbSsoAssertionStore, SsoAssertionStoreError};
use crate::credential_contracts::byok_cache::SystemClock;
use crate::sso_assertions::assertion::{SsoIdentityAssertion, decode_payload};
use crate::sso_assertions::cache::{CacheState, SsoAssertionCache};

const SALT_KEY: &str = "b10011-salt-key";
const WARNING_PREFIX: &str = "Failed to persist the SSO identity assertion for EMA egress";
const EXPIRES_AT: i64 = 1_893_456_000;

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

fn failures(upsert: Option<usize>, id_jag_server_exists: Option<usize>) -> MemoryFailures {
    MemoryFailures {
        find: None,
        find_all: None,
        upsert,
        update: None,
        id_jag_server_exists,
    }
}

fn memory_rows(id_jag: bool, failures: MemoryFailures) -> MemoryAssertionRows {
    MemoryAssertionRows {
        rows: Mutex::new(Vec::new()),
        id_jag,
        failures,
        reads: AtomicUsize::new(0),
        writes: AtomicUsize::new(0),
    }
}

fn store_with(
    rows: Option<MemoryAssertionRows>,
    cache: Arc<SsoAssertionCache>,
    config_declares_id_jag: bool,
) -> DbSsoAssertionStore<MemoryAssertionRows> {
    DbSsoAssertionStore {
        rows,
        cipher: cipher_with(Some(SALT_KEY)),
        cache,
        config_declares_id_jag,
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

fn stored_assertion(rows: &MemoryAssertionRows) -> Option<SsoIdentityAssertion> {
    let assertion_b64 = rows
        .rows
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .map(|row| row.assertion_b64.clone())?;
    let stored = StoredValue::<Infallible>::Text(SecretString::from(assertion_b64));
    let opened = cipher_with(Some(SALT_KEY)).decrypt_value(
        stored,
        ASSERTION_DECRYPT_FIELD,
        ASSERTION_DECRYPT_POLICY,
    )?;
    match opened {
        StoredValue::Text(plaintext) => decode_payload(plaintext.expose_secret()).ok(),
        StoredValue::Other(never) => match never {},
    }
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<(String, String)>>>);

impl Logs {
    fn messages(&self) -> Vec<String> {
        let entries = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        entries.iter().map(|(_, message)| message.clone()).collect()
    }
}

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
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
            .push(("WARNING".to_owned(), message.0));
    }
}

#[tokio::test]
async fn persist_writes_the_encrypted_payload_and_then_invalidates_the_cache() {
    let cache = empty_cache();
    cache.set_if_unchanged("alice", &assertion("cached-token", None), cache.epoch());
    let store = store_with(
        Some(memory_rows(false, MemoryFailures::default())),
        Arc::clone(&cache),
        false,
    );
    store
        .persist("alice", &assertion("live-token", Some("refresh-token")))
        .await
        .unwrap();
    assert!(cache.get("alice").is_none());
    let rows = store.rows.as_ref().unwrap();
    assert_eq!(rows.writes.load(Ordering::SeqCst), 1);
    assert_eq!(rows.reads.load(Ordering::SeqCst), 0);
    let stored = stored_assertion(rows).unwrap();
    assert_eq!(stored.id_token.expose_secret(), "live-token");
    assert_eq!(
        stored.refresh_token.unwrap().expose_secret(),
        "refresh-token"
    );
    assert_eq!(stored.issuer.as_deref(), Some("https://idp.test"));
    assert_eq!(stored.expires_at, DateTime::from_timestamp(EXPIRES_AT, 0));
}

#[tokio::test]
async fn a_failed_upsert_leaves_the_cache_alone() {
    let cache = empty_cache();
    cache.set_if_unchanged("alice", &assertion("cached-token", None), cache.epoch());
    let store = store_with(
        Some(memory_rows(false, failures(Some(0), None))),
        Arc::clone(&cache),
        false,
    );
    let result = store.persist("alice", &assertion("live-token", None)).await;
    assert!(matches!(result, Err(SsoAssertionStoreError::Store(_))));
    assert_eq!(
        cached_token(&cache, "alice").as_deref(),
        Some("cached-token")
    );
    let rows = store.rows.as_ref().unwrap();
    assert_eq!(rows.writes.load(Ordering::SeqCst), 0);
    assert!(rows.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn persist_without_a_database_does_nothing() {
    let cache = empty_cache();
    cache.set_if_unchanged("alice", &assertion("cached-token", None), cache.epoch());
    let store = DbSsoAssertionStore::<MemoryAssertionRows> {
        rows: None,
        // a keyless cipher proves the early return runs before any encryption
        cipher: cipher_with(None),
        cache: Arc::clone(&cache),
        config_declares_id_jag: false,
    };
    store
        .persist("alice", &assertion("live-token", None))
        .await
        .unwrap();
    assert_eq!(
        cached_token(&cache, "alice").as_deref(),
        Some("cached-token")
    );
}

#[tokio::test]
async fn an_encryption_failure_is_reported_without_a_write() {
    let store = DbSsoAssertionStore::<MemoryAssertionRows> {
        rows: Some(memory_rows(false, MemoryFailures::default())),
        cipher: cipher_with(None),
        cache: empty_cache(),
        config_declares_id_jag: false,
    };
    let result = store.persist("alice", &assertion("live-token", None)).await;
    assert!(matches!(result, Err(SsoAssertionStoreError::Cipher(_))));
    let rows = store.rows.as_ref().unwrap();
    assert_eq!(rows.writes.load(Ordering::SeqCst), 0);
    assert!(rows.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn retention_is_enabled_from_the_config_without_a_query() {
    let store = store_with(
        Some(memory_rows(false, MemoryFailures::default())),
        empty_cache(),
        true,
    );
    assert!(store.retention_enabled().await.unwrap());
    assert_eq!(store.rows.as_ref().unwrap().reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn retention_is_disabled_without_a_database() {
    let store = DbSsoAssertionStore::<MemoryAssertionRows> {
        rows: None,
        cipher: cipher_with(Some(SALT_KEY)),
        cache: empty_cache(),
        config_declares_id_jag: false,
    };
    assert!(!store.retention_enabled().await.unwrap());
}

#[rstest]
#[case::row_present(true)]
#[case::row_absent(false)]
#[tokio::test]
async fn retention_follows_the_row_lookup(#[case] id_jag: bool) {
    let store = store_with(
        Some(memory_rows(id_jag, MemoryFailures::default())),
        empty_cache(),
        false,
    );
    assert_eq!(store.retention_enabled().await.unwrap(), id_jag);
    assert_eq!(store.rows.as_ref().unwrap().reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retention_reports_a_row_failure() {
    let store = store_with(
        Some(memory_rows(false, failures(None, Some(0)))),
        empty_cache(),
        false,
    );
    let result = store.retention_enabled().await;
    assert!(matches!(result, Err(SsoAssertionStoreError::Store(_))));
}

#[tokio::test]
async fn retain_does_nothing_for_a_missing_assertion() {
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    let store = store_with(
        Some(memory_rows(true, MemoryFailures::default())),
        empty_cache(),
        true,
    );
    store.retain("alice", None).await;
    let rows = store.rows.as_ref().unwrap();
    assert_eq!(rows.reads.load(Ordering::SeqCst), 0);
    assert_eq!(rows.writes.load(Ordering::SeqCst), 0);
    assert!(logs.messages().is_empty());
}

#[tokio::test]
async fn retain_does_nothing_when_the_gate_is_closed() {
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    let store = store_with(
        Some(memory_rows(false, MemoryFailures::default())),
        empty_cache(),
        false,
    );
    store
        .retain("alice", Some(&assertion("live-token", None)))
        .await;
    let rows = store.rows.as_ref().unwrap();
    assert_eq!(rows.reads.load(Ordering::SeqCst), 1);
    assert_eq!(rows.writes.load(Ordering::SeqCst), 0);
    assert!(logs.messages().is_empty());
}

#[rstest]
#[case::gate(failures(None, Some(0)), false)]
#[case::write(failures(Some(0), None), true)]
#[tokio::test]
async fn retain_swallows_a_failure_and_logs_the_warning(
    #[case] memory_failures: MemoryFailures,
    #[case] config_declares_id_jag: bool,
) {
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    let store = store_with(
        Some(memory_rows(true, memory_failures)),
        empty_cache(),
        config_declares_id_jag,
    );
    store
        .retain("alice", Some(&assertion("live-token", None)))
        .await;
    let messages = logs.messages();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].starts_with(&format!("{WARNING_PREFIX} (user_id=alice): ")));
    assert_eq!(
        store.rows.as_ref().unwrap().writes.load(Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn retain_persists_when_the_gate_is_open() {
    let store = store_with(
        Some(memory_rows(false, MemoryFailures::default())),
        empty_cache(),
        true,
    );
    store
        .retain(
            "alice",
            Some(&assertion("live-token", Some("refresh-token"))),
        )
        .await;
    let rows = store.rows.as_ref().unwrap();
    assert_eq!(rows.reads.load(Ordering::SeqCst), 0);
    assert_eq!(rows.writes.load(Ordering::SeqCst), 1);
    let stored = stored_assertion(rows).unwrap();
    assert_eq!(stored.id_token.expose_secret(), "live-token");
}

#[tokio::test]
async fn no_token_text_reaches_logs_errors_or_debug_output() {
    let secret = assertion("sk-live-id-token-9f31", Some("refresh-token-9f31"));
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    let cache = empty_cache();
    cache.set_if_unchanged("alice", &secret, cache.epoch());
    let cached = cache.get("alice");
    assert!(!format!("{cached:?}").contains("sk-live-id-token-9f31"));
    assert!(!format!("{cached:?}").contains("refresh-token-9f31"));
    let store = store_with(
        Some(memory_rows(false, failures(Some(0), None))),
        Arc::clone(&cache),
        true,
    );
    let error = store.persist("alice", &secret).await.unwrap_err();
    store.retain("alice", Some(&secret)).await;
    assert!(!error.to_string().contains("sk-live-id-token-9f31"));
    assert!(!format!("{error:?}").contains("sk-live-id-token-9f31"));
    assert!(!format!("{error:?}").contains("refresh-token-9f31"));
    let messages = logs.messages().join(" ");
    assert!(!messages.contains("sk-live-id-token-9f31"));
    assert!(!messages.contains("refresh-token-9f31"));
}
