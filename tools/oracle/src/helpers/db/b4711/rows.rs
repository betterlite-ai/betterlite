use std::sync::{Mutex, PoisonError};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp::table_access::filter::{FindServers, ServerFilter};
use betterlite_db::mcp::table_access::rows::{RowFields, ServerUpdate};
use betterlite_db::mcp::table_access::types::{ACTIVE, McpServerAccess, McpTableError};

// An in-memory stand-in for the Prisma MCP server table: it answers the same filters in table order
pub(super) struct RowStore {
    rows: Mutex<Vec<JsonData>>,
}

impl RowStore {
    pub(super) fn new(rows: &[JsonData]) -> Self {
        Self {
            rows: Mutex::new(rows.to_vec()),
        }
    }

    fn select(&self, request: &FindServers) -> Vec<JsonData> {
        let mut selected: Vec<JsonData> = self
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|stored| {
                request
                    .filter
                    .as_ref()
                    .is_none_or(|filter| matches(stored, filter))
            })
            .cloned()
            .collect();
        if request.descending {
            selected.sort_by(|left, right| {
                text(right, "submitted_at").cmp(&text(left, "submitted_at"))
            });
        }
        if let Some(take) = request.take {
            selected.truncate(usize::try_from(take).unwrap_or(usize::MAX));
        }
        selected
    }

    fn update(&self, request: &ServerUpdate) -> Result<Option<JsonData>, McpTableError> {
        let mut stored_rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        stored_rows
            .iter_mut()
            .find(|stored| text(stored, "server_id") == Some(request.server_id.as_str()))
            .map(|stored| apply(stored, &request.data).map(|()| stored.clone()))
            .transpose()
    }
}

impl McpServerAccess for RowStore {
    fn find_servers(
        &self,
        request: &FindServers,
    ) -> impl Future<Output = Result<Vec<JsonData>, McpTableError>> + Send {
        std::future::ready(Ok(self.select(request)))
    }

    fn update_server(
        &self,
        request: &ServerUpdate,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send {
        std::future::ready(self.update(request))
    }
}

// Prisma's insensitive mode lowercases both sides, as LOWER() does in the betterlite query
fn matches(stored: &JsonData, filter: &ServerFilter) -> bool {
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
            let wanted = value.to_lowercase();
            let named = ["server_name", "alias"]
                .iter()
                .any(|field| text(stored, field).is_some_and(|name| name.to_lowercase() == wanted));
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

fn apply(stored: &mut JsonData, data: &RowFields) -> Result<(), McpTableError> {
    let JsonData::Object(fields) = stored else {
        return Ok(());
    };
    for (name, value) in &data.0 {
        let encoded = serde_json::to_string(value).map_err(McpTableError::Json)?;
        let decoded = serde_json::from_str(&encoded).map_err(McpTableError::Json)?;
        fields.insert((*name).to_owned(), decoded);
    }
    Ok(())
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
