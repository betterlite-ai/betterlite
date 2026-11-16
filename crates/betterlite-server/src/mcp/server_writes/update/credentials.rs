use betterlite_db::mcp_server_encoding::credentials::{
    CredentialBlob, credentials_blob_to_dict, drop_stale_minted_tokens,
};
use betterlite_db::mcp_server_encoding::request::PreparedServerData;
use betterlite_db::mcp_stored_contracts::JsonValue;

use super::super::ServerWriteError;
use super::existing::{ExistingRow, TOKEN_EXCHANGE_COLUMNS};
use super::identity::IdentityChanges;

pub(super) fn apply_credential_merge(
    changes: &IdentityChanges,
    existing: Option<&ExistingRow>,
    data: &mut PreparedServerData,
) -> Result<(), ServerWriteError> {
    let Some(row) = existing else {
        return Ok(());
    };
    let Some(stored) = row.credentials.as_ref() else {
        return Ok(());
    };
    let explicit_te = TOKEN_EXCHANGE_COLUMNS
        .iter()
        .any(|field| data.contains_key(*field));
    match data.get("credentials") {
        // New values override the stored ones and the stored keys the update omits survive, so a
        // partial credential edit never wipes the encrypted secrets the UI cannot show again.
        Some(JsonValue::String(new_text)) if !changes.auth_type => {
            let new_blob = credentials_blob_to_dict(&JsonValue::String(new_text.clone()))?;
            let mut merged = credentials_blob_to_dict(stored)?;
            for (field, value) in &new_blob {
                merged.insert(field.clone(), value.clone());
            }
            drop_stale_minted_tokens(&mut merged, &new_blob);
            lift_token_exchange(row, &mut merged, data);
            store_blob(data, &merged)
        }
        // An explicit null, or a replacement that a cross-class switch writes as given.
        Some(_) => Ok(()),
        // A column write that does not touch credentials still strips the legacy blob copies, so a
        // cleared column can never be repopulated by a stale copy on the next credentials write.
        _ if explicit_te => {
            let mut blob = credentials_blob_to_dict(stored)?;
            if !TOKEN_EXCHANGE_COLUMNS
                .iter()
                .any(|field| blob.contains_key(*field))
            {
                return Ok(());
            }
            lift_token_exchange(row, &mut blob, data);
            store_blob(data, &blob)
        }
        _ => Ok(()),
    }
}

fn lift_token_exchange(
    existing: &ExistingRow,
    blob: &mut CredentialBlob,
    data: &mut PreparedServerData,
) {
    let columns = [
        (
            "token_exchange_endpoint",
            existing.token_exchange_endpoint.as_deref(),
        ),
        ("audience", existing.audience.as_deref()),
        ("subject_token_type", existing.subject_token_type.as_deref()),
        (
            "token_exchange_profile",
            existing.token_exchange_profile.as_deref(),
        ),
    ];
    for (field, column) in columns {
        let Some(legacy) = blob.remove(field) else {
            continue;
        };
        if column.is_none() && !data.contains_key(field) {
            data.insert(field.to_owned(), legacy);
        }
    }
}

fn store_blob(
    data: &mut PreparedServerData,
    blob: &CredentialBlob,
) -> Result<(), ServerWriteError> {
    let text = serde_json::to_string(blob).map_err(ServerWriteError::Json)?;
    data.insert("credentials".to_owned(), JsonValue::String(text));
    Ok(())
}
