#![cfg(test)]

use std::convert::Infallible;
use std::fmt::Debug;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::sso_assertion_rows::{
    ASSERTION_DECRYPT_FIELD, ASSERTION_DECRYPT_POLICY, AssertionRow, MemoryAssertionRows,
    MemoryFailures,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, OsNonces, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::{RotationError, rotate_assertions};

const OLD_KEY: &str = "old-salt-key";
const NEW_KEY: &str = "new-salt-key";

fn cipher(key: &str) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(key.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn sealed(text: &str) -> String {
    let stored = StoredValue::<Infallible>::Text(SecretString::from(text.to_owned()));
    let ciphertext = match cipher(OLD_KEY).encrypt_value(stored, None).unwrap() {
        StoredValue::Text(ciphertext) => ciphertext,
        StoredValue::Other(never) => match never {},
    };
    ciphertext.expose_secret().to_owned()
}

fn opened(text: &str, key: &str) -> Option<String> {
    let stored = StoredValue::<Infallible>::Text(SecretString::from(text.to_owned()));
    let policy = ASSERTION_DECRYPT_POLICY;
    match cipher(key).decrypt_value(stored, ASSERTION_DECRYPT_FIELD, policy) {
        Some(StoredValue::Text(plaintext)) => Some(plaintext.expose_secret().to_owned()),
        Some(StoredValue::Other(never)) => match never {},
        None => None,
    }
}

fn row(user_id: &str, assertion_b64: &str) -> AssertionRow {
    AssertionRow {
        user_id: user_id.to_owned(),
        assertion_b64: assertion_b64.to_owned(),
    }
}

fn sealed_rows(entries: &[(&str, &str)]) -> Vec<AssertionRow> {
    entries
        .iter()
        .map(|(user_id, payload)| row(user_id, &sealed(payload)))
        .collect()
}

fn memory_rows(stored: Vec<AssertionRow>, failures: MemoryFailures) -> MemoryAssertionRows {
    MemoryAssertionRows {
        rows: Mutex::new(stored),
        failures,
        ..MemoryAssertionRows::default()
    }
}

fn kept_rows(rows: &MemoryAssertionRows) -> Vec<AssertionRow> {
    rows.rows
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<(String, String)>>>);

impl Logs {
    fn entries(&self) -> Vec<(String, String)> {
        let entries = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        entries.clone()
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
        let level = *event.metadata().level();
        if level != Level::WARN && level != Level::INFO {
            return;
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        let name = if level == Level::WARN {
            "WARNING"
        } else {
            "INFO"
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((name.to_owned(), message.0));
    }
}

#[tokio::test]
async fn every_readable_row_moves_to_the_new_key() {
    let old = cipher(OLD_KEY);
    let key = SecretString::from(NEW_KEY);
    let entries = [("alpha", "first-payload"), ("bravo", "second-payload")];
    let rows = memory_rows(sealed_rows(&entries), MemoryFailures::default());
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    rotate_assertions(&rows, &old, &key).await.unwrap();
    let kept = kept_rows(&rows);
    assert_eq!(kept.len(), 2);
    assert_eq!(
        opened(&kept[0].assertion_b64, NEW_KEY).as_deref(),
        Some("first-payload")
    );
    assert_eq!(opened(&kept[0].assertion_b64, OLD_KEY), None);
    assert_eq!(
        opened(&kept[1].assertion_b64, NEW_KEY).as_deref(),
        Some("second-payload")
    );
    assert_eq!(opened(&kept[1].assertion_b64, OLD_KEY), None);
    assert_eq!(rows.reads.load(Ordering::SeqCst), 1, "one find_all");
    assert_eq!(rows.writes.load(Ordering::SeqCst), 2);
    assert_eq!(
        logs.entries(),
        [(
            "INFO".to_owned(),
            "rotate_sso_identity_assertions_master_key: rotated 2 row(s), skipped 0".to_owned(),
        )]
    );
}

#[tokio::test]
async fn skipped_row_keeps_its_text_and_is_counted() {
    let old = cipher(OLD_KEY);
    let key = SecretString::from(NEW_KEY);
    let mut seeded = sealed_rows(&[("alpha", "first-payload"), ("charlie", "third-payload")]);
    seeded.insert(1, row("bravo", "not-a-ciphertext"));
    let rows = memory_rows(seeded, MemoryFailures::default());
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    rotate_assertions(&rows, &old, &key).await.unwrap();
    let kept = kept_rows(&rows);
    assert_eq!(
        opened(&kept[0].assertion_b64, NEW_KEY).as_deref(),
        Some("first-payload")
    );
    assert_eq!(
        kept[1].assertion_b64, "not-a-ciphertext",
        "the skipped row keeps its text"
    );
    assert_eq!(
        opened(&kept[2].assertion_b64, NEW_KEY).as_deref(),
        Some("third-payload")
    );
    assert_eq!(
        logs.entries(),
        [
            (
                "WARNING".to_owned(),
                "rotate_sso_identity_assertions_master_key: could not decrypt assertion for user_id=bravo, skipping".to_owned(),
            ),
            (
                "INFO".to_owned(),
                "rotate_sso_identity_assertions_master_key: rotated 2 row(s), skipped 1".to_owned(),
            ),
        ]
    );
    assert_eq!(rows.writes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn empty_table_logs_zero_counts() {
    let old = cipher(OLD_KEY);
    let key = SecretString::from(NEW_KEY);
    let rows = memory_rows(Vec::new(), MemoryFailures::default());
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    rotate_assertions(&rows, &old, &key).await.unwrap();
    assert_eq!(
        logs.entries(),
        [(
            "INFO".to_owned(),
            "rotate_sso_identity_assertions_master_key: rotated 0 row(s), skipped 0".to_owned(),
        )]
    );
    assert_eq!(rows.reads.load(Ordering::SeqCst), 1);
    assert_eq!(rows.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn update_failure_stops_the_rotation() {
    let old = cipher(OLD_KEY);
    let key = SecretString::from(NEW_KEY);
    let entries = [
        ("alpha", "first-payload"),
        ("bravo", "second-payload"),
        ("charlie", "third-payload"),
    ];
    let failures = MemoryFailures {
        update: Some(1),
        ..MemoryFailures::default()
    };
    let rows = memory_rows(sealed_rows(&entries), failures);
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    let result = rotate_assertions(&rows, &old, &key).await;
    assert!(matches!(result, Err(RotationError::Store(_))));
    let kept = kept_rows(&rows);
    assert_eq!(
        opened(&kept[0].assertion_b64, NEW_KEY).as_deref(),
        Some("first-payload"),
        "the row before the failure stays rotated"
    );
    assert_eq!(
        opened(&kept[1].assertion_b64, OLD_KEY).as_deref(),
        Some("second-payload"),
        "the failing row keeps the old key"
    );
    assert_eq!(
        opened(&kept[2].assertion_b64, OLD_KEY).as_deref(),
        Some("third-payload"),
        "the row after the failure is never touched"
    );
    assert!(logs.entries().is_empty(), "no summary after a failure");
    assert_eq!(rows.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn plaintext_never_reaches_logs_or_errors() {
    let old = cipher(OLD_KEY);
    let key = SecretString::from(NEW_KEY);
    let secret = "sk-live-assertion-token-4f2c";
    let ciphertext = sealed(secret);
    let seeded = vec![row("alpha", &ciphertext), row("bravo", "raw-token-text")];
    let rows = memory_rows(seeded, MemoryFailures::default());
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    rotate_assertions(&rows, &old, &key).await.unwrap();
    let text: String = logs
        .entries()
        .iter()
        .map(|(_, message)| message.as_str())
        .collect();
    assert!(!text.contains(secret), "no plaintext in a log line");
    assert!(!text.contains(&ciphertext), "no ciphertext in a log line");
    assert!(
        !text.contains("raw-token-text"),
        "no stored text in a log line"
    );
    let failing = memory_rows(
        vec![row("charlie", &sealed(secret))],
        MemoryFailures {
            update: Some(0),
            ..MemoryFailures::default()
        },
    );
    let error = rotate_assertions(&failing, &old, &key).await.unwrap_err();
    assert!(
        !error.to_string().contains(secret),
        "no plaintext in the error"
    );
}
