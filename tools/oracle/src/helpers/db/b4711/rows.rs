use std::sync::{Mutex, PoisonError};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp::table_access::filter::{FindServers, ServerFilter};
use betterlite_db::mcp::table_access::rows::{RowFields, ServerUpdate};
use betterlite_db::mcp::table_access::types::{ACTIVE, McpServerAccess, McpTableError};

// An in-memory stand-in for the Prisma MCP server table: it answers the same filters in table order
pub(super) struct RowStore {
    pub(super) rows: Mutex<Vec<JsonData>>,
}

impl McpServerAccess for RowStore {
    async fn find_servers(&self, request: &FindServers) -> Result<Vec<JsonData>, McpTableError> {
        let mut selected = {
            let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
            let mut matched = Vec::new();
            for stored in &*rows {
                if request
                    .filter
                    .as_ref()
                    .is_none_or(|filter| matches(stored, filter))
                {
                    matched.push(stored.clone());
                }
            }
            matched
        };
        if request.descending {
            selected.sort_by(|left, right| {
                text(right, "submitted_at").cmp(&text(left, "submitted_at"))
            });
        }
        if let Some(take) = request.take {
            selected.truncate(usize::try_from(take).unwrap_or(usize::MAX));
        }
        Ok(selected)
    }

    async fn update_server(
        &self,
        request: &ServerUpdate,
    ) -> Result<Option<JsonData>, McpTableError> {
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        for stored in &mut *rows {
            if text(stored, "server_id") == Some(request.server_id.as_str()) {
                apply(stored, &request.data)?;
                return Ok(Some(stored.clone()));
            }
        }
        Ok(None)
    }
}

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
            let named = ["server_name", "alias"].iter().any(|field| {
                text(stored, field).is_some_and(|name| name.eq_ignore_ascii_case(value))
            });
            let excluded = exclude_server_id
                .as_deref()
                .is_some_and(|excluded| text(stored, "server_id") == Some(excluded));
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
    let Some(value) = fields.get(name) else {
        return None;
    };
    match value {
        JsonData::String(value) => Some(value),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    }
}
