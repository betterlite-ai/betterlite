use std::future::ready;
use std::sync::Mutex;

use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use secrecy::SecretString;

use crate::budget_permission_rows::JsonData;
use crate::mcp::table_access::filter::{FindServers, ServerFilter};
use crate::mcp::table_access::rows::{RowFields, ServerUpdate};
use crate::mcp::table_access::types::{ACTIVE, McpServerAccess, McpTableError};
use crate::mcp_stored_contracts::JsonValue;
use crate::stored_cipher::DbCipher;

pub(super) fn cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("test-salt")),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(super) fn row(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the fake row is valid JSON")
}

// An in-memory stand-in for the Prisma table: it answers the same where clauses from rows in table order
pub(super) struct FakeStore {
    rows: Mutex<Vec<JsonData>>,
    updates: Mutex<Vec<ServerUpdate>>,
    finds: Mutex<Vec<FindServers>>,
}

impl FakeStore {
    pub(super) const fn new(rows: Vec<JsonData>) -> Self {
        Self {
            rows: Mutex::new(rows),
            updates: Mutex::new(Vec::new()),
            finds: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn updates(&self) -> Vec<ServerUpdate> {
        self.updates
            .lock()
            .expect("the updates are not poisoned")
            .clone()
    }

    pub(super) fn finds(&self) -> Vec<FindServers> {
        self.finds
            .lock()
            .expect("the finds are not poisoned")
            .clone()
    }

    fn select(&self, request: &FindServers) -> Vec<JsonData> {
        let mut selected: Vec<JsonData> = self
            .rows
            .lock()
            .expect("the rows are not poisoned")
            .iter()
            .filter(|stored| {
                request
                    .filter
                    .as_ref()
                    .is_none_or(|filter| filter_matches(stored, filter))
            })
            .cloned()
            .collect();
        if request.descending {
            selected.sort_by(|left, right| submitted_at(right).cmp(&submitted_at(left)));
        }
        if let Some(take) = request.take {
            selected.truncate(usize::try_from(take).expect("the take limit fits in usize"));
        }
        selected
    }

    fn update(&self, request: &ServerUpdate) -> Option<JsonData> {
        self.updates
            .lock()
            .expect("the updates are not poisoned")
            .push(request.clone());
        let mut stored_rows = self.rows.lock().expect("the rows are not poisoned");
        stored_rows
            .iter_mut()
            .find(|stored| text(stored, "server_id") == Some(request.server_id.as_str()))
            .map(|stored| {
                apply(stored, &request.data);
                stored.clone()
            })
    }
}

impl McpServerAccess for FakeStore {
    fn find_servers(
        &self,
        request: &FindServers,
    ) -> impl Future<Output = Result<Vec<JsonData>, McpTableError>> + Send {
        self.finds
            .lock()
            .expect("the finds are not poisoned")
            .push(request.clone());
        ready(Ok(self.select(request)))
    }

    fn update_server(
        &self,
        request: &ServerUpdate,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send {
        ready(Ok(self.update(request)))
    }
}

fn apply(stored: &mut JsonData, data: &RowFields) {
    let JsonData::Object(fields) = stored else {
        return;
    };
    for (name, value) in &data.0 {
        fields.insert((*name).to_owned(), data_of(value));
    }
}

fn data_of(value: &JsonValue) -> JsonData {
    let text = serde_json::to_string(value).expect("a row field serializes");
    serde_json::from_str(&text).expect("a row field is JSON")
}

fn filter_matches(stored: &JsonData, filter: &ServerFilter) -> bool {
    match filter {
        ServerFilter::ServerId(server_id) => text(stored, "server_id") == Some(server_id.as_str()),
        ServerFilter::ServerIds(server_ids) => {
            let stored_id = text(stored, "server_id");
            server_ids
                .iter()
                .any(|wanted| stored_id == Some(wanted.as_str()))
        }
        ServerFilter::Identifiers {
            value,
            exclude_server_id,
        } => {
            let columns = [text(stored, "server_name"), text(stored, "alias")];
            let named = columns
                .iter()
                .any(|candidate| candidate.is_some_and(|name| name.eq_ignore_ascii_case(value)));
            let excluded = exclude_server_id
                .as_deref()
                .is_some_and(|own_id| text(stored, "server_id") == Some(own_id));
            named && !excluded && text(stored, "approval_status") != Some("draft")
        }
        ServerFilter::ApprovalStatus(status) => {
            text(stored, "approval_status") == Some(status.as_str())
        }
        ServerFilter::NotDraft => text(stored, "approval_status") != Some("draft"),
        ServerFilter::SubmittedByActive { user_id } => {
            text(stored, "submitted_by") == Some(user_id.as_str())
                && text(stored, "approval_status") == Some(ACTIVE)
        }
        ServerFilter::Submitted => text(stored, "submitted_at").is_some(),
    }
}

fn submitted_at(stored: &JsonData) -> Option<&str> {
    text(stored, "submitted_at")
}

fn text<'row>(stored: &'row JsonData, name: &str) -> Option<&'row str> {
    let JsonData::Object(fields) = stored else {
        return None;
    };
    match fields.get(name)? {
        JsonData::String(value) => Some(value),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    }
}
