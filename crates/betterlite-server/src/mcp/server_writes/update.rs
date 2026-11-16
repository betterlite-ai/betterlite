mod credentials;
mod existing;
mod identity;
mod write;

use betterlite_db::mcp::table_access::types::LockedWriteOutcome;
use betterlite_db::mcp_server_encoding::prepare::prepare_mcp_server_data;
use betterlite_db::mcp_server_encoding::request::{
    FieldsSet, McpServerRequest, PrepareMode, PreparedServerData,
};
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::stored_cipher::DbCipher;

use self::credentials::apply_credential_merge;
use self::existing::{ExistingRow, TOKEN_EXCHANGE_COLUMNS};
use self::identity::{IdentityChanges, apply_identity_changes};
use self::write::write_update_row;
use super::decode::{decode_server_row, row_as};
use super::prepare::row_fields;
use super::{ServerWriteError, ServerWriteStore, UpdatedServer};

// LiteLLM's update_mcp_server: exclude_unset partial update, then identity, credential and audit writes.
pub async fn update_mcp_server<S>(
    store: &S,
    cipher: &DbCipher,
    request: &McpServerRequest,
    touched_by: &str,
    fields_set: &FieldsSet,
) -> Result<UpdatedServer, ServerWriteError>
where
    S: ServerWriteStore,
{
    let Some(server_id) = request.server_id.clone() else {
        return Err(ServerWriteError::MissingServerId);
    };
    let mut data = prepare_update_data(request, fields_set, cipher)?;
    let existing = load_existing(store, &server_id, request, &data).await?;
    let changes = IdentityChanges::detect(request, existing.as_ref(), &data);
    apply_identity_changes(&changes, existing.as_ref(), &mut data);
    apply_credential_merge(&changes, existing.as_ref(), &mut data)?;
    data.insert(
        "updated_by".to_owned(),
        JsonValue::String(touched_by.to_owned()),
    );
    let written = write_update_row(store, &server_id, row_fields(data)?).await?;
    match written {
        LockedWriteOutcome::Conflict(conflict) => Ok(UpdatedServer::Conflict(conflict)),
        LockedWriteOutcome::Written(Some(row)) => {
            decode_server_row(row, cipher).map(|server| UpdatedServer::Updated(Some(server)))
        }
        LockedWriteOutcome::Written(None) => Ok(UpdatedServer::Updated(None)),
    }
}

fn prepare_update_data(
    request: &McpServerRequest,
    fields_set: &FieldsSet,
    cipher: &DbCipher,
) -> Result<PreparedServerData, ServerWriteError> {
    // The normalizing pass always assigns alias, so it is set on the model even when the caller
    // omitted it; the caller's own set is what tells an omitted alias from an explicit null.
    let mut model_fields = fields_set.clone();
    model_fields.insert("alias".to_owned());
    let mode = PrepareMode::Update {
        model_fields: &model_fields,
        original_fields: fields_set,
    };
    Ok(prepare_mcp_server_data(request, mode, cipher)?)
}

async fn load_existing<S>(
    store: &S,
    server_id: &str,
    request: &McpServerRequest,
    data: &PreparedServerData,
) -> Result<Option<ExistingRow>, ServerWriteError>
where
    S: ServerWriteStore,
{
    let has_credentials = data
        .get("credentials")
        .is_some_and(|value| !matches!(value, JsonValue::Null));
    // An explicit token-exchange column write also migrates the legacy blob copies.
    let explicit_te = TOKEN_EXCHANGE_COLUMNS
        .iter()
        .any(|field| data.contains_key(*field));
    let url_provided = data
        .get("url")
        .is_some_and(|value| !matches!(value, JsonValue::Null));
    let issuer_provided = data.contains_key("issuer");
    if !(request.auth_type.is_some()
        || has_credentials
        || explicit_te
        || url_provided
        || issuer_provided)
    {
        return Ok(None);
    }
    match store.find_server(server_id).await? {
        Some(row) => Ok(Some(row_as(&row)?)),
        None => Ok(None),
    }
}
