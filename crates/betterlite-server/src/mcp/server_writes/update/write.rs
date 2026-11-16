use betterlite_db::mcp::table_access::rows::{RowFields, ServerUpdate};
use betterlite_db::mcp::table_access::server_reads::identifier_field;
use betterlite_db::mcp::table_access::types::{LockedWrite, LockedWriteOutcome, ServerWrite};
use betterlite_db::mcp_stored_contracts::JsonValue;

use super::super::decode::row_as;
use super::super::{ServerWriteError, ServerWriteStore};
use super::existing::ExistingRow;

// LiteLLM's _update_mcp_server_row: an identifier rewrite takes the advisory locks, the rest does not.
pub(super) async fn write_update_row<S>(
    store: &S,
    server_id: &str,
    fields: RowFields,
) -> Result<LockedWriteOutcome, ServerWriteError>
where
    S: ServerWriteStore,
{
    if !fields.0.contains_key("server_name") && !fields.0.contains_key("alias") {
        let update = ServerUpdate {
            server_id: server_id.to_owned(),
            data: fields,
        };
        return Ok(LockedWriteOutcome::Written(
            store.update_server(update).await?,
        ));
    }
    if let Some(clearing) = alias_clear_write(store, server_id, &fields).await? {
        return Ok(store.write_if_identifier_free(clearing).await?);
    }
    let request = LockedWrite {
        server_name: identifier_field(&fields, "server_name"),
        alias: identifier_field(&fields, "alias"),
        exclude_server_id: Some(server_id.to_owned()),
        write: ServerWrite::Update(ServerUpdate {
            server_id: server_id.to_owned(),
            data: fields,
        }),
    };
    Ok(store.write_if_identifier_free(request).await?)
}

// Clearing the alias drops the prefix to the stored server_name, which may already belong to
// another row, so that name has to be locked and checked too.
async fn alias_clear_write<S>(
    store: &S,
    server_id: &str,
    fields: &RowFields,
) -> Result<Option<LockedWrite>, ServerWriteError>
where
    S: ServerWriteStore,
{
    if fields.0.contains_key("server_name") {
        return Ok(None);
    }
    let alias_set =
        matches!(fields.0.get("alias"), Some(JsonValue::String(text)) if !text.is_empty());
    if alias_set {
        return Ok(None);
    }
    let Some(row) = store.find_server(server_id).await? else {
        return Ok(None);
    };
    let existing: ExistingRow = row_as(&row)?;
    Ok(Some(LockedWrite {
        server_name: existing.server_name,
        alias: None,
        exclude_server_id: Some(server_id.to_owned()),
        write: ServerWrite::Update(ServerUpdate {
            server_id: server_id.to_owned(),
            data: fields.clone(),
        }),
    }))
}
