use betterlite_db::mcp_server_encoding::blank::blank_to_none;
use betterlite_db::mcp_server_encoding::credentials::credential_auth_class;
use betterlite_db::mcp_server_encoding::request::{McpServerRequest, PreparedServerData};
use betterlite_db::mcp_stored_contracts::JsonValue;

use super::existing::{AUTH_FLOW_SCOPED_FIELDS, ExistingRow};

// Which mint-relevant identity values this update changes, decided once from the stored row.
pub(super) struct IdentityChanges {
    pub(super) auth_type: bool,
    // A new url or issuer re-points the server at another upstream, so its discovered endpoints expire.
    pub(super) upstream: bool,
}

impl IdentityChanges {
    pub(super) fn detect(
        request: &McpServerRequest,
        existing: Option<&ExistingRow>,
        data: &PreparedServerData,
    ) -> Self {
        let Some(row) = existing else {
            return Self {
                auth_type: false,
                upstream: false,
            };
        };
        // A switch within the client-forwarded class keeps the same declared app, so it is no change.
        let auth_type = request.auth_type.is_some()
            && credential_auth_class(row.auth_type) != credential_auth_class(request.auth_type);
        let url = data.get("url").is_some_and(|value| match value {
            JsonValue::String(submitted) => row.url.as_deref() != Some(submitted.as_str()),
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_) => false,
        });
        let old_issuer = blank_to_none(row.issuer.as_deref());
        let issuer = data.get("issuer").is_some_and(|value| {
            let submitted = match value {
                JsonValue::String(text) => blank_to_none(Some(text.as_str())),
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_) => None,
            };
            old_issuer.is_some() && submitted != old_issuer
        });
        Self {
            auth_type,
            upstream: url || issuer,
        }
    }
}

pub(super) fn apply_identity_changes(
    changes: &IdentityChanges,
    existing: Option<&ExistingRow>,
    data: &mut PreparedServerData,
) {
    // A cross-class auth_type switch replaces the stored credentials entirely.
    if changes.auth_type && !data.contains_key("credentials") {
        data.insert("credentials".to_owned(), JsonValue::Null);
    }
    if !(changes.auth_type || changes.upstream) {
        return;
    }
    if let Some(row) = existing {
        // The edit form re-sends every field, so only a value that differs from the stored one survives.
        for field in AUTH_FLOW_SCOPED_FIELDS {
            let unchanged = data
                .get(field)
                .is_none_or(|value| row.stored_field(field).as_ref() == Some(value));
            if unchanged {
                data.insert(field.to_owned(), JsonValue::Null);
            }
        }
    }
}
