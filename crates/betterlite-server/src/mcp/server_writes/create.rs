use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::server_reads::identifier_field;
use betterlite_db::mcp::table_access::types::{LockedWrite, LockedWriteOutcome, ServerWrite};
use betterlite_db::mcp_server_encoding::request::McpServerRequest;
use betterlite_db::stored_cipher::DbCipher;
use uuid::Uuid;

use super::decode::decode_server_row;
use super::prepare::create_fields;
use super::{CreatedServer, ServerWriteError, ServerWriteStore};

// LiteLLM's create_mcp_server: generate the id when the caller gave none, insert, read the row back.
pub async fn create_mcp_server<S>(
    store: &S,
    cipher: &DbCipher,
    request: &McpServerRequest,
    touched_by: &str,
) -> Result<McpServerTable, ServerWriteError>
where
    S: ServerWriteStore,
{
    let identified = with_server_id(request);
    let data = create_fields(&identified, touched_by, cipher)?;
    let row = store.create_server(data).await?;
    decode_server_row(row, cipher)
}

// ast-grep-ignore: no-long-names
pub async fn create_mcp_server_if_identifier_free<S>(
    store: &S,
    cipher: &DbCipher,
    request: &McpServerRequest,
    touched_by: &str,
) -> Result<CreatedServer, ServerWriteError>
where
    S: ServerWriteStore,
{
    let identified = with_server_id(request);
    let data = create_fields(&identified, touched_by, cipher)?;
    let locked = LockedWrite {
        server_name: identifier_field(&data, "server_name"),
        alias: identifier_field(&data, "alias"),
        exclude_server_id: None,
        write: ServerWrite::Create(data),
    };
    match store.write_if_identifier_free(locked).await? {
        LockedWriteOutcome::Conflict(conflict) => Ok(CreatedServer::Conflict(Box::new(conflict))),
        LockedWriteOutcome::Written(Some(row)) => {
            let server = decode_server_row(row, cipher)?;
            Ok(CreatedServer::Created(Box::new(server)))
        }
        LockedWriteOutcome::Written(None) => Err(ServerWriteError::InsertedRowMissing),
    }
}

fn with_server_id(request: &McpServerRequest) -> McpServerRequest {
    let mut identified = request.clone();
    if identified.server_id.is_none() {
        identified.server_id = Some(Uuid::new_v4().to_string());
    }
    identified
}
