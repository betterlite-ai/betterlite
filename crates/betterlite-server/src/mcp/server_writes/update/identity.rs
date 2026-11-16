use betterlite_db::mcp_server_encoding::blank::blank_to_none;
use betterlite_db::mcp_server_encoding::credentials::credential_auth_class;
use betterlite_db::mcp_server_encoding::request::{McpServerRequest, PreparedServerData};
use betterlite_db::mcp_stored_contracts::JsonValue;

use super::existing::{AUTH_FLOW_SCOPED_FIELDS, ExistingRow};

// Which mint-relevant identity values this update changes, decided once from the stored row.
pub(super) struct IdentityChanges {
    pub(super) auth_type: bool,
    pub(super) url: bool,
    pub(super) issuer: bool,
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
                url: false,
                issuer: false,
            };
        };
        // A switch within the client-forwarded class keeps the same declared app, so it is no change.
        let auth_type = request.auth_type.is_some()
            && credential_auth_class(row.auth_type) != credential_auth_class(request.auth_type);
        // A url change re-points the server at another upstream, so every discovered endpoint expires.
        let url = match data.get("url") {
            Some(JsonValue::String(url)) => row.url.as_deref() != Some(url.as_str()),
            _ => false,
        };
        let issuer = match (data.get("issuer"), blank_to_none(row.issuer.as_deref())) {
            (Some(JsonValue::String(text)), Some(old)) => {
                blank_to_none(Some(text.as_str())) != Some(old)
            }
            (Some(_), Some(_)) => true,
            _ => false,
        };
        Self {
            auth_type,
            url,
            issuer,
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
    if !(changes.auth_type || changes.url || changes.issuer) {
        return;
    }
    if let Some(row) = existing {
        // The edit form re-sends every field, so a value equal to the stored one is stale too;
        // only a genuinely new submitted value survives.
        for field in AUTH_FLOW_SCOPED_FIELDS {
            let unchanged = match data.get(field) {
                None | Some(JsonValue::Null) => true,
                Some(value) => row.stored_field(field).as_ref() == Some(value),
            };
            if unchanged {
                data.insert(field.to_owned(), JsonValue::Null);
            }
        }
    }
}
