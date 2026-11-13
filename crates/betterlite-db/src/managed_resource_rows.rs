pub mod autorouter;
pub mod files;
pub mod fixtures;
pub mod mcp;
pub mod skills;
pub mod spend_logs;
pub mod tag;
pub mod wire;

use std::collections::BTreeMap;
use std::str::{Utf8Error, from_utf8};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::budget_permission_rows::JsonData;
use crate::mcp_stored_contracts::pinned::PinnedError;
use crate::stored_cipher::DbCipher;
use autorouter::AutoRouterSession;
use betterlite_secrets::error::SecretError;
use files::{
    ManagedFileTable, ManagedObjectTable, ManagedVectorStoreTable, ManagedVectorStoresTable,
};
use mcp::{McpEnvVar, McpServerTable};
use skills::SkillTable;
use spend_logs::{ErrorLog, SpendLog};
use tag::TagTable;

const MAX_ROW_BYTES: usize = 0x0010_0000;
const MAX_JSON_DEPTH: usize = 64;

pub struct ManagedRowDependencies<'cipher> {
    pub cipher: &'cipher DbCipher,
    pub error_request_id: Uuid,
}

#[derive(Debug, thiserror::Error)]
pub enum ManagedRowsDecodeError {
    #[error("stored managed row is {len} bytes, over the {MAX_ROW_BYTES} byte limit")]
    TooLarge { len: usize },
    #[error("stored managed row is not valid UTF-8")]
    InvalidUtf8(#[source] Utf8Error),
    #[error("stored managed row exceeds JSON depth {MAX_JSON_DEPTH}")]
    TooDeep,
    #[error("stored managed row is invalid JSON")]
    Json(#[source] serde_json::Error),
    #[error("stored managed row input needs to be a JSON object")]
    NotAnObject,
    #[error("stored managed row kind {kind} is not known")]
    UnknownKind { kind: String },
    #[error("field {field} must be a JSON string")]
    JsonFieldNotString { field: &'static str },
    #[error(transparent)]
    SecretMap(#[from] SecretError),
    #[error(transparent)]
    PinnedTools(#[from] PinnedError),
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
#[serde(tag = "kind", content = "row")]
pub enum ManagedResourceRow<
    File = JsonData,
    Batch = JsonData,
    FineTuning = JsonData,
    Response = JsonData,
> {
    #[serde(rename = "managed_file")]
    ManagedFile(Box<ManagedFileTable<File>>),
    #[serde(rename = "managed_object")]
    ManagedObject(Box<ManagedObjectTable<Batch, FineTuning, Response>>),
    #[serde(rename = "managed_vector_store")]
    ManagedVectorStore(Box<ManagedVectorStoreTable>),
    #[serde(rename = "managed_vector_stores")]
    ManagedVectorStores(Box<ManagedVectorStoresTable>),
    #[serde(rename = "mcp_env_var")]
    McpEnvVar(Box<McpEnvVar>),
    #[serde(rename = "mcp_server")]
    McpServer(Box<McpServerTable>),
    #[serde(rename = "skill")]
    Skill(Box<SkillTable>),
    #[serde(rename = "autorouter_session")]
    AutoRouterSession(Box<AutoRouterSession>),
    #[serde(rename = "spend_log")]
    SpendLog(Box<SpendLog>),
    #[serde(rename = "error_log")]
    ErrorLog(Box<ErrorLog>),
    #[serde(rename = "tag")]
    Tag(Box<TagTable>),
}

#[derive(Deserialize)]
struct StoredEnvelope {
    kind: String,
    row: JsonData,
}

pub fn decode_managed_resource_rows<File, Batch, FineTuning, Response>(
    bytes: &[u8],
    dependencies: &ManagedRowDependencies<'_>,
) -> Result<ManagedResourceRow<File, Batch, FineTuning, Response>, ManagedRowsDecodeError>
where
    File: DeserializeOwned,
    Batch: DeserializeOwned,
    FineTuning: DeserializeOwned,
    Response: DeserializeOwned,
{
    if bytes.len() > MAX_ROW_BYTES {
        return Err(ManagedRowsDecodeError::TooLarge { len: bytes.len() });
    }
    let text = from_utf8(bytes).map_err(ManagedRowsDecodeError::InvalidUtf8)?;
    check_depth(text)?;
    let mut envelope =
        serde_json::from_str::<StoredEnvelope>(text).map_err(ManagedRowsDecodeError::Json)?;
    decode_typed_row(&envelope.kind, &mut envelope.row, dependencies)
}

fn decode_typed_row<File, Batch, FineTuning, Response>(
    kind: &str,
    row: &mut JsonData,
    dependencies: &ManagedRowDependencies<'_>,
) -> Result<ManagedResourceRow<File, Batch, FineTuning, Response>, ManagedRowsDecodeError>
where
    File: DeserializeOwned,
    Batch: DeserializeOwned,
    FineTuning: DeserializeOwned,
    Response: DeserializeOwned,
{
    let decoded = match kind {
        "managed_file" => ManagedResourceRow::ManagedFile(Box::new(decode_row(row)?)),
        "managed_object" => ManagedResourceRow::ManagedObject(Box::new(decode_row(row)?)),
        "managed_vector_store" => {
            ManagedResourceRow::ManagedVectorStore(Box::new(decode_row(row)?))
        }
        "managed_vector_stores" => {
            ManagedResourceRow::ManagedVectorStores(Box::new(decode_row(row)?))
        }
        "mcp_env_var" => ManagedResourceRow::McpEnvVar(Box::new(decode_row(row)?)),
        "mcp_server" => {
            ManagedResourceRow::McpServer(Box::new(mcp::decode_server(row, dependencies.cipher)?))
        }
        "skill" => ManagedResourceRow::Skill(Box::new(decode_row(row)?)),
        "autorouter_session" => ManagedResourceRow::AutoRouterSession(Box::new(decode_row(row)?)),
        "spend_log" => ManagedResourceRow::SpendLog(Box::new(spend_logs::decode_spend_log(row)?)),
        "error_log" => ManagedResourceRow::ErrorLog(Box::new(spend_logs::decode_error_log(
            row,
            dependencies.error_request_id,
        )?)),
        "tag" => ManagedResourceRow::Tag(Box::new(tag::decode_tag(row)?)),
        other => {
            return Err(ManagedRowsDecodeError::UnknownKind {
                kind: other.to_owned(),
            });
        }
    };
    Ok(decoded)
}

pub(crate) fn decode_row<T>(row: &JsonData) -> Result<T, ManagedRowsDecodeError>
where
    T: DeserializeOwned,
{
    serde_json::to_value(row)
        .and_then(serde_json::from_value)
        .map_err(ManagedRowsDecodeError::Json)
}

pub(crate) const fn object_fields(
    row: &mut JsonData,
) -> Result<&mut BTreeMap<String, JsonData>, ManagedRowsDecodeError> {
    match row {
        JsonData::Object(fields) => Ok(fields),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => Err(ManagedRowsDecodeError::NotAnObject),
    }
}

#[derive(Clone, Copy)]
enum ScanState {
    Structural,
    Quoted,
    Escaped,
}

pub(crate) fn check_depth(text: &str) -> Result<(), ManagedRowsDecodeError> {
    let mut state = ScanState::Structural;
    let mut depth = 0_u8;
    for byte in text.bytes() {
        (state, depth) = advance_depth_scan(state, byte, depth)?;
    }
    Ok(())
}

fn advance_depth_scan(
    state: ScanState,
    byte: u8,
    depth: u8,
) -> Result<(ScanState, u8), ManagedRowsDecodeError> {
    match (state, byte) {
        (ScanState::Structural, b'{' | b'[') => {
            let next_depth = depth.saturating_add(1);
            if usize::from(next_depth) > MAX_JSON_DEPTH {
                return Err(ManagedRowsDecodeError::TooDeep);
            }
            Ok((ScanState::Structural, next_depth))
        }
        (ScanState::Structural, b'}' | b']') => {
            Ok((ScanState::Structural, depth.saturating_sub(1)))
        }
        (ScanState::Structural, b'"') | (ScanState::Escaped, _) => Ok((ScanState::Quoted, depth)),
        (ScanState::Quoted, b'\\') => Ok((ScanState::Escaped, depth)),
        (ScanState::Quoted, b'"') => Ok((ScanState::Structural, depth)),
        (ScanState::Structural | ScanState::Quoted, _) => Ok((state, depth)),
    }
}

#[path = "managed_resource_rows/tests.rs"]
mod tests;
