use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, OsNonces, StoredCipher, StoredValue,
};
use chrono::{DateTime, TimeDelta, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Builder;
use tracing::dispatcher::with_default;
use tracing::field::{Field, Visit};
use tracing::subscriber::Subscriber;
use tracing::{Dispatch, Level};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::budget_permission_rows::JsonData;
use crate::mcp::table_access::types::McpTableError;
use crate::mcp_stored_contracts::JsonValue;
use crate::mcp_user_credentials::rows::{CredentialRows, CredentialScope, text_field};
use crate::mcp_user_credentials::types::OAuthCredentialPayload;
use crate::oauth_token_resolution::ports::TokenResponse;
use crate::oauth_token_resolution::purge::PurgeRows;
use crate::search_tools::convert::isoformat;
use crate::stored_cipher::DbCipher;

pub(super) const UPDATED_AT: &str = "2024-01-02T03:04:05+00:00";

pub(super) const UNDECRYPTABLE: &str = "_w==";

pub(super) const BYOK_KEY: &str = "sk-user-owned-key";

pub(super) fn cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("b4715-salt")),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(super) fn sealed(cipher: &DbCipher, text: &str) -> String {
    let value = StoredValue::<()>::Text(SecretString::from(text));
    let StoredValue::Text(sealed) = cipher
        .encrypt_value(value, None)
        .expect("the test cipher encrypts")
    else {
        panic!("a text value stays text");
    };
    sealed.expose_secret().to_owned()
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

pub(super) fn now() -> DateTime<Utc> {
    moment(UPDATED_AT)
}

pub(super) fn expires_at(seconds: i64) -> String {
    let offset = TimeDelta::try_seconds(seconds).expect("the test offset fits");
    isoformat(
        now()
            .checked_add_signed(offset)
            .expect("the test moment fits"),
    )
}

pub(super) fn payload(text: &str) -> OAuthCredentialPayload {
    serde_json::from_str(text).expect("the test payload is valid")
}

pub(super) fn token_response(entries: &[(&str, JsonValue)]) -> TokenResponse {
    entries
        .iter()
        .map(|(field, value)| ((*field).to_owned(), value.clone()))
        .collect()
}

pub(super) fn text_response(entries: &[(&str, &str)]) -> TokenResponse {
    entries
        .iter()
        .map(|(field, value)| ((*field).to_owned(), JsonValue::String((*value).to_owned())))
        .collect()
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

pub(super) fn oauth_row(
    cipher: &DbCipher,
    user_id: &str,
    server_id: &str,
    payload_text: &str,
) -> JsonData {
    credential_row(user_id, server_id, &sealed(cipher, payload_text))
}

pub(super) fn byok_row(cipher: &DbCipher, user_id: &str, server_id: &str) -> JsonData {
    credential_row(user_id, server_id, &sealed(cipher, BYOK_KEY))
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

// The in-memory stand-in for the credential table: rows are matched by pair, and the mode chooses a failure the caller must survive
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum TableMode {
    #[default]
    Normal,
    FailReads,
    FailWrites,
    SkipDeletes,
}

pub(super) struct FakeTable {
    rows: Mutex<Vec<JsonData>>,
    mode: TableMode,
    read_override: Option<JsonData>,
    read_calls: Mutex<Vec<(String, String)>>,
    delete_calls: Mutex<Vec<(String, String)>>,
}

impl FakeTable {
    pub(super) fn new(rows: Vec<JsonData>) -> Self {
        Self {
            rows: Mutex::new(rows),
            mode: TableMode::Normal,
            read_override: None,
            read_calls: Mutex::new(Vec::new()),
            delete_calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn in_mode(mode: TableMode) -> Self {
        Self {
            rows: Mutex::new(Vec::new()),
            mode,
            read_override: None,
            read_calls: Mutex::new(Vec::new()),
            delete_calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn reading(mut self, row: JsonData) -> Self {
        self.read_override = Some(row);
        self
    }

    pub(super) fn with_mode(mut self, mode: TableMode) -> Self {
        self.mode = mode;
        self
    }

    pub(super) fn stored(&self) -> Vec<JsonData> {
        self.rows
            .lock()
            .expect("the fake rows are not poisoned")
            .clone()
    }

    pub(super) fn reads(&self) -> Vec<(String, String)> {
        self.read_calls
            .lock()
            .expect("the fake reads are not poisoned")
            .clone()
    }

    pub(super) fn deletes(&self) -> Vec<(String, String)> {
        self.delete_calls
            .lock()
            .expect("the fake deletes are not poisoned")
            .clone()
    }

    fn find(&self, user_id: &str, server_id: &str) -> Option<JsonData> {
        if let Some(row) = &self.read_override {
            return Some(row.clone());
        }
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

    fn record_read(&self, user_id: &str, server_id: &str) {
        self.read_calls
            .lock()
            .expect("the fake reads are not poisoned")
            .push((user_id.to_owned(), server_id.to_owned()));
    }

    fn record_delete(&self, user_id: &str, server_id: &str) {
        self.delete_calls
            .lock()
            .expect("the fake deletes are not poisoned")
            .push((user_id.to_owned(), server_id.to_owned()));
    }

    fn read_outcome<T>(&self, loaded: T) -> Result<T, McpTableError> {
        if self.mode == TableMode::FailReads {
            return Err(McpTableError::MissingServerId);
        }
        Ok(loaded)
    }

    fn select_scope(&self, scope: CredentialScope<'_>) -> Vec<JsonData> {
        match scope {
            CredentialScope::User(user_id) => self.select("user_id", user_id),
            CredentialScope::Server(server_id) => self.select("server_id", server_id),
        }
    }

    fn remove_pair(&self, user_id: &str, server_id: &str) -> usize {
        let mut rows = self.rows.lock().expect("the fake rows are not poisoned");
        let before = rows.len();
        rows.retain(|stored| !is_pair(stored, user_id, server_id));
        before.saturating_sub(rows.len())
    }

    fn delete_pairs(&self, server_id: &str, user_ids: &[String]) -> usize {
        let mut deleted = 0_usize;
        for user_id in user_ids {
            self.record_delete(user_id, server_id);
            if self.mode != TableMode::SkipDeletes {
                deleted = deleted.saturating_add(self.remove_pair(user_id, server_id));
            }
        }
        deleted
    }
}

impl CredentialRows for FakeTable {
    fn find_credential(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send {
        self.record_read(user_id, server_id);
        ready(self.read_outcome(self.find(user_id, server_id)))
    }

    fn find_credentials(
        &self,
        scope: CredentialScope<'_>,
    ) -> impl Future<Output = Result<Vec<JsonData>, McpTableError>> + Send {
        ready(self.read_outcome(self.select_scope(scope)))
    }

    #[rustfmt::skip]
    fn upsert_credential(&self, user_id: &str, server_id: &str, credential_b64: &str) -> impl Future<Output = Result<(), McpTableError>> + Send {
        if self.mode == TableMode::FailWrites {
            return ready(Err(McpTableError::MissingServerId));
        }
        self.upsert(user_id, server_id, credential_b64);
        ready(Ok(()))
    }

    #[rustfmt::skip]
    fn delete_credential(&self, user_id: &str, server_id: &str) -> impl Future<Output = Result<(), McpTableError>> + Send {
        self.rows.lock().expect("the fake rows are not poisoned").retain(|stored| !is_pair(stored, user_id, server_id));
        ready(Ok(()))
    }
}

impl PurgeRows for FakeTable {
    #[rustfmt::skip]
    fn delete_credential_pairs(&self, server_id: &str, user_ids: &[String]) -> impl Future<Output = Result<usize, McpTableError>> + Send {
        ready(Ok(self.delete_pairs(server_id, user_ids)))
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
