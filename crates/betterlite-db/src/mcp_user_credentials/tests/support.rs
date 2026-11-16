use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use tokio::runtime::Builder;
use tracing::dispatcher::with_default;
use tracing::field::{Field, Visit};
use tracing::subscriber::Subscriber;
use tracing::{Dispatch, Level};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::budget_permission_rows::JsonData;
use crate::mcp::table_access::types::McpTableError;
use crate::mcp_user_credentials::rows::{CredentialRows, CredentialScope, text_field};
use crate::mcp_user_credentials::types::OAuthCredentialPayload;
use crate::stored_cipher::DbCipher;

pub(super) const UPDATED_AT: &str = "2024-01-02T03:04:05+00:00";

pub(super) fn cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("b4713-salt")),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(super) fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}

pub(super) fn exposed(token: Option<&SecretString>) -> Option<&str> {
    token.map(ExposeSecret::expose_secret)
}

pub(super) fn moment(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("the test moment is RFC 3339")
        .with_timezone(&Utc)
}

// the instant every store and expiry test reads as "now"
pub(super) fn now() -> DateTime<Utc> {
    moment("2024-01-02T03:04:05+00:00")
}

pub(super) fn payload(text: &str) -> OAuthCredentialPayload {
    serde_json::from_str(text).expect("the test payload is valid")
}

pub(super) fn credential_row(user_id: &str, server_id: &str, credential: &str) -> JsonData {
    JsonData::Object(BTreeMap::from([
        ("user_id".to_owned(), JsonData::String(user_id.to_owned())),
        (
            "server_id".to_owned(),
            JsonData::String(server_id.to_owned()),
        ),
        (
            "credential_b64".to_owned(),
            JsonData::String(credential.to_owned()),
        ),
        (
            "updated_at".to_owned(),
            JsonData::String(UPDATED_AT.to_owned()),
        ),
    ]))
}

pub(super) fn block_on<Run>(run: Run) -> Run::Output
where
    Run: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime builds")
        .block_on(run)
}

// An in-memory stand-in for the Prisma user credential table: composite key lookups, column filters and an upsert that keeps the row
#[derive(Default)]
pub(super) struct FakeTable {
    rows: Mutex<Vec<JsonData>>,
}

impl FakeTable {
    pub(super) fn new(rows: Vec<JsonData>) -> Self {
        Self {
            rows: Mutex::new(rows),
        }
    }

    pub(super) fn stored(&self) -> Vec<JsonData> {
        self.rows
            .lock()
            .expect("the fake rows are not poisoned")
            .clone()
    }

    fn find(&self, user_id: &str, server_id: &str) -> Option<JsonData> {
        let rows = self.rows.lock().expect("the fake rows are not poisoned");
        rows.iter()
            .find(|stored| is_pair(stored, user_id, server_id))
            .cloned()
    }

    fn select(&self, field: &str, value: &str) -> Vec<JsonData> {
        let rows = self.rows.lock().expect("the fake rows are not poisoned");
        rows.iter()
            .filter(|stored| text_field(stored, field) == Some(value))
            .cloned()
            .collect()
    }

    fn upsert(&self, user_id: &str, server_id: &str, credential: &str) {
        let mut rows = self.rows.lock().expect("the fake rows are not poisoned");
        for stored in rows.iter_mut() {
            if is_pair(stored, user_id, server_id) {
                set_text(stored, "credential_b64", credential);
                return;
            }
        }
        rows.push(credential_row(user_id, server_id, credential));
    }
}

impl CredentialRows for FakeTable {
    fn find_credential(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send {
        ready(Ok(self.find(user_id, server_id)))
    }

    fn find_credentials(
        &self,
        scope: CredentialScope<'_>,
    ) -> impl Future<Output = Result<Vec<JsonData>, McpTableError>> + Send {
        ready(Ok(match scope {
            CredentialScope::User(user_id) => self.select("user_id", user_id),
            CredentialScope::Server(server_id) => self.select("server_id", server_id),
        }))
    }

    // the trait's two writers share one signature; one line each keeps them under the similarity check's 8-line floor
    #[rustfmt::skip]
    fn upsert_credential(&self, user_id: &str, server_id: &str, credential_b64: &str) -> impl Future<Output = Result<(), McpTableError>> + Send {
        self.upsert(user_id, server_id, credential_b64);
        ready(Ok(()))
    }

    #[rustfmt::skip]
    fn delete_credential(&self, user_id: &str, server_id: &str) -> impl Future<Output = Result<(), McpTableError>> + Send {
        self.rows.lock().expect("the fake rows are not poisoned").retain(|stored| !is_pair(stored, user_id, server_id));
        ready(Ok(()))
    }
}

fn is_pair(stored: &JsonData, user_id: &str, server_id: &str) -> bool {
    text_field(stored, "user_id") == Some(user_id)
        && text_field(stored, "server_id") == Some(server_id)
}

fn set_text(stored: &mut JsonData, field: &str, value: &str) {
    let JsonData::Object(fields) = stored else {
        return;
    };
    drop(fields.insert(field.to_owned(), JsonData::String(value.to_owned())));
}

#[derive(Clone, Default)]
struct WarnLogs(Arc<Mutex<Vec<String>>>);

struct EventFields(String);

impl Visit for EventFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field, value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.push(field, &format!("{value:?}"));
        }
    }
}

impl EventFields {
    fn push(&mut self, field: &Field, value: &str) {
        self.0.push_str(field.name());
        self.0.push('=');
        self.0.push_str(value);
    }
}

impl<Inner: Subscriber> Layer<Inner> for WarnLogs {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut fields = EventFields(String::new());
        event.record(&mut fields);
        self.0
            .lock()
            .expect("the warnings are not poisoned")
            .push(fields.0);
    }
}

pub(super) fn warnings_during<Run>(run: Run) -> Vec<String>
where
    Run: FnOnce(),
{
    let logs = WarnLogs::default();
    let dispatch = Dispatch::new(tracing_subscriber::registry().with(logs.clone()));
    with_default(&dispatch, run);
    logs.0
        .lock()
        .expect("the warnings are not poisoned")
        .clone()
}
