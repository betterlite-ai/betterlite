use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::McpIdentifierConflict;
use betterlite_db::mcp_server_encoding::credentials::{
    credentials_blob_to_dict, decrypt_credentials,
};
use betterlite_db::mcp_server_encoding::request::{FieldsSet, McpServerRequest};
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use betterlite_server::mcp::server_writes::create::create_mcp_server;
// ast-grep-ignore: no-long-names
use betterlite_server::mcp::server_writes::create::create_mcp_server_if_identifier_free;
use betterlite_server::mcp::server_writes::decode::decode_server_row;
use betterlite_server::mcp::server_writes::delete::delete_mcp_server;
use betterlite_server::mcp::server_writes::draft::{create_draft_mcp_server, get_draft_mcp_server};
use betterlite_server::mcp::server_writes::update::update_mcp_server;
use betterlite_server::mcp::server_writes::{CreatedServer, DraftOptions, UpdatedServer};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::runtime::Builder;

mod store;

use self::store::{MemoryStore, RecordingInvalidation};

pub(crate) struct Op;

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Function {
    Create,
    CreateIfFree,
    CreateDraft,
    GetDraft,
    Update,
    Delete,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    function: Function,
    salt_key: String,
    touched_by: String,
    #[serde(default)]
    payload: Value,
    #[serde(default)]
    rows: Vec<JsonData>,
    #[serde(default)]
    server_id: String,
    #[serde(default)]
    ttl_seconds: i64,
    #[serde(default)]
    users: Vec<String>,
}

#[derive(Serialize)]
struct Answer {
    server_id: Option<String>,
    conflict: Option<Value>,
    rows: Vec<Value>,
    invalidated: Vec<String>,
    deleted_children: Vec<&'static str>,
}

// Everything one helper call works against, built once from the call.
struct Session {
    cipher: DbCipher,
    store: MemoryStore,
    invalidated: RecordingInvalidation,
    request: McpServerRequest,
    fields: FieldsSet,
}

// The server_id a call answers with and the identifier collision it reported, if any.
type Outcome = Result<(Option<String>, Option<Value>), Box<HelperError>>;

impl Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        Builder::new_current_thread()
            .build()
            .or_value_error()?
            .block_on(run_call(input))
    }
}

fn cipher(salt: &str) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(salt.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

async fn run_call(call: &Call) -> HelperResult {
    let (request, fields) = split_payload(&call.payload)?;
    let session = Session {
        cipher: cipher(&call.salt_key),
        store: MemoryStore::new(&call.rows, &call.users, &call.server_id),
        invalidated: RecordingInvalidation::new(),
        request,
        fields,
    };
    let (server_id, conflict) = match call.function {
        Function::Create => create(call, &session).await?,
        Function::CreateIfFree => create_if_free(call, &session).await?,
        Function::CreateDraft => create_draft(call, &session).await?,
        Function::GetDraft => get_draft(call, &session).await?,
        Function::Update => update(call, &session).await?,
        Function::Delete => delete(call, &session).await?,
    };
    let answer = Answer {
        server_id,
        conflict,
        rows: rows_view(&session.store, &session.cipher)?,
        invalidated: session.invalidated.calls(),
        deleted_children: session.store.deleted_children(),
    };
    serde_json::to_value(answer).or_value_error()
}

async fn create(call: &Call, session: &Session) -> Outcome {
    let server = create_mcp_server(
        &session.store,
        &session.cipher,
        &session.request,
        &call.touched_by,
    )
    .await
    .or_value_error()?;
    Ok((Some(server.server_id), None))
}

async fn create_if_free(call: &Call, session: &Session) -> Outcome {
    // ast-grep-ignore: no-long-names
    let created = create_mcp_server_if_identifier_free(
        &session.store,
        &session.cipher,
        &session.request,
        &call.touched_by,
    )
    .await
    .or_value_error()?;
    match created {
        CreatedServer::Created(server) => Ok((Some(server.server_id), None)),
        CreatedServer::Conflict(conflict) => Ok((None, Some(conflict_view(&conflict)))),
    }
}

async fn create_draft(call: &Call, session: &Session) -> Outcome {
    let options = DraftOptions {
        touched_by: call.touched_by.clone(),
        ttl_seconds: call.ttl_seconds,
        server_id: (!call.server_id.is_empty()).then_some(call.server_id.clone()),
    };
    let draft = create_draft_mcp_server(
        &session.store,
        &session.cipher,
        &session.request,
        &options,
        &session.invalidated,
    )
    .await
    .or_value_error()?;
    Ok((Some(draft.server_id), None))
}

async fn get_draft(call: &Call, session: &Session) -> Outcome {
    let found = get_draft_mcp_server(
        &session.store,
        &session.cipher,
        &call.server_id,
        call.ttl_seconds,
    )
    .await
    .or_value_error()?;
    Ok((found.map(|draft| draft.server_id), None))
}

async fn update(call: &Call, session: &Session) -> Outcome {
    let updated = update_mcp_server(
        &session.store,
        &session.cipher,
        &session.request,
        &call.touched_by,
        &session.fields,
    )
    .await
    .or_value_error()?;
    match updated {
        UpdatedServer::Updated(server) => Ok((server.map(|row| row.server_id), None)),
        UpdatedServer::Conflict(conflict) => Ok((None, Some(conflict_view(&conflict)))),
    }
}

async fn delete(call: &Call, session: &Session) -> Outcome {
    let deleted = delete_mcp_server(&session.store, &call.server_id, &session.invalidated)
        .await
        .or_value_error()?;
    Ok((deleted.map(|gone| gone.server_id), None))
}

// The update path needs the caller's own field set, which the typed request no longer carries.
fn split_payload(payload: &Value) -> Result<(McpServerRequest, FieldsSet), Box<HelperError>> {
    let request = serde_json::from_value(payload.clone()).or_value_error()?;
    let mut fields = FieldsSet::new();
    if let Some(object) = payload.as_object() {
        for name in object.keys() {
            fields.insert(name.clone());
        }
    }
    Ok((request, fields))
}

fn conflict_view(conflict: &McpIdentifierConflict) -> Value {
    json!({
        "field": conflict.field.as_str(),
        "value": conflict.value,
        "server_id": conflict.server_id,
    })
}

fn rows_view(store: &MemoryStore, cipher: &DbCipher) -> Result<Vec<Value>, Box<HelperError>> {
    let mut views = Vec::new();
    for row in store.rows() {
        let server = decode_server_row(row, cipher).or_value_error()?;
        views.push(server_view(&server, cipher)?);
    }
    Ok(views)
}

fn server_view(server: &McpServerTable, cipher: &DbCipher) -> HelperResult {
    let credentials = match server.credentials.as_ref() {
        Some(stored_blob) => {
            let value = serde_json::to_value(stored_blob).or_value_error()?;
            let stored: JsonValue = serde_json::from_value(value).or_value_error()?;
            let mut blob = credentials_blob_to_dict(&stored).or_value_error()?;
            decrypt_credentials(&mut blob, cipher);
            serde_json::to_value(blob).or_value_error()?
        }
        None => Value::Null,
    };
    Ok(json!({
        "server_id": server.server_id,
        "server_name": server.server_name,
        "alias": server.alias,
        "description": server.description,
        "auth_type": server.auth_type,
        "approval_status": server.approval_status,
        "created_by": server.created_by,
        "updated_by": server.updated_by,
        "is_byok": server.is_byok,
        "credentials": credentials,
        "env_vars": server.env_vars,
        "issuer": server.issuer,
        "token_url": server.token_url,
        "oauth2_flow": server.oauth2_flow,
        "token_exchange_endpoint": server.token_exchange_endpoint,
        "audience": server.audience,
    }))
}
