use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::McpIdentifierConflict;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::HelperError;

use super::helper_error;

pub(super) fn cipher(salt: &str) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(salt.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(super) fn conflict_value(conflict: Option<McpIdentifierConflict>) -> Value {
    let Some(conflict) = conflict else {
        return Value::Null;
    };
    json!({
        "field": conflict.field.as_str(),
        "value": conflict.value,
        "server_id": conflict.server_id,
    })
}

pub(super) fn server_view(server: &McpServerTable) -> Result<Value, Box<HelperError>> {
    let env = serde_json::to_value(&server.env)
        .map_err(|source| helper_error("SerializationError", &source.to_string()))?;
    Ok(json!({
        "server_id": server.server_id,
        "approval_status": server.approval_status,
        "env": env,
    }))
}

pub(super) fn review_view(server: &McpServerTable) -> Value {
    json!({
        "server": {
            "server_id": server.server_id,
            "approval_status": server.approval_status,
            "review_notes": server.review_notes,
            "updated_by": server.updated_by,
            "reviewed_at_present": server.reviewed_at.is_some(),
        }
    })
}
