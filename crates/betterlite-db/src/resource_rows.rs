pub mod access_group;
pub mod config;
pub mod credential;
pub mod model;
pub mod organization;
pub mod project;

use std::str::{Utf8Error, from_utf8};

use access_group::AccessGroupTable;
use config::ConfigRow;
use credential::{CreateCredentialItem, CredentialBase, CredentialItem, UpdateCredentialItem};
use model::ProxyModelTable;
use organization::OrganizationTable;
use project::ProjectTable;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::JsonData;

const MAX_ROW_BYTES: usize = 0x0010_0000;
const MAX_JSON_DEPTH: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum ResourceRowsError {
    #[error("stored row is {len} bytes, over the {MAX_ROW_BYTES} byte limit")]
    TooLarge { len: usize },
    #[error("stored row is not valid UTF-8")]
    InvalidUtf8(#[source] Utf8Error),
    #[error("stored row exceeds JSON depth {MAX_JSON_DEPTH}")]
    TooDeep,
    #[error("stored row is invalid JSON")]
    Json(#[source] serde_json::Error),
    #[error("input needs to be a dictionary")]
    NotAnObject,
    #[error("Either credential_values or model_id must be set")]
    CredentialValueRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "row")]
pub enum ResourceRow {
    #[serde(rename = "access_group")]
    AccessGroup(AccessGroupTable),
    #[serde(rename = "config")]
    Config(ConfigRow),
    #[serde(rename = "credential_base")]
    CredentialBase(CredentialBase),
    #[serde(rename = "credential_item")]
    CredentialItem(CredentialItem),
    #[serde(rename = "credential_create")]
    CredentialCreate(Box<CreateCredentialItem>),
    #[serde(rename = "credential_update")]
    CredentialUpdate(UpdateCredentialItem),
    #[serde(rename = "model")]
    Model(Box<ProxyModelTable>),
    #[serde(rename = "organization")]
    Organization(Box<OrganizationTable>),
    #[serde(rename = "project")]
    Project(Box<ProjectTable>),
}

pub fn decode_resource_rows(bytes: &[u8]) -> Result<ResourceRow, ResourceRowsError> {
    let value = parse_data(bytes)?;
    convert(&value)
}

pub(crate) fn parse_data(bytes: &[u8]) -> Result<JsonData, ResourceRowsError> {
    if bytes.len() > MAX_ROW_BYTES {
        return Err(ResourceRowsError::TooLarge { len: bytes.len() });
    }
    let text = from_utf8(bytes).map_err(ResourceRowsError::InvalidUtf8)?;
    check_depth(text)?;
    serde_json::from_str(text).map_err(ResourceRowsError::Json)
}

pub(crate) fn convert<T>(value: &JsonData) -> Result<T, ResourceRowsError>
where
    T: DeserializeOwned,
{
    let text = serde_json::to_string(value).map_err(ResourceRowsError::Json)?;
    if text.len() > MAX_ROW_BYTES {
        return Err(ResourceRowsError::TooLarge { len: text.len() });
    }
    check_depth(&text)?;
    serde_json::from_str(&text).map_err(ResourceRowsError::Json)
}

pub(crate) fn is_truthy(input: &JsonData) -> bool {
    match input {
        JsonData::Null => false,
        JsonData::Bool(flag) => *flag,
        JsonData::Number(number) => number.as_f64().is_some_and(|parsed| parsed != 0.0_f64),
        JsonData::String(text) => !text.is_empty(),
        JsonData::Array(items) => !items.is_empty(),
        JsonData::Object(fields) => !fields.is_empty(),
    }
}

#[derive(Clone, Copy)]
enum ScanState {
    Structural,
    Quoted,
    Escaped,
}

fn check_depth(text: &str) -> Result<(), ResourceRowsError> {
    let mut state = ScanState::Structural;
    let mut depth = 0_u8;
    for byte in text.bytes() {
        match (state, byte) {
            (ScanState::Structural, b'{' | b'[') => {
                depth = depth.saturating_add(1);
                if usize::from(depth) > MAX_JSON_DEPTH {
                    return Err(ResourceRowsError::TooDeep);
                }
            }
            (ScanState::Structural, b'}' | b']') => depth = depth.saturating_sub(1),
            (ScanState::Structural, b'"') | (ScanState::Escaped, _) => state = ScanState::Quoted,
            (ScanState::Quoted, b'\\') => state = ScanState::Escaped,
            (ScanState::Quoted, b'"') => state = ScanState::Structural,
            (ScanState::Structural | ScanState::Quoted, _) => {}
        }
    }
    Ok(())
}

#[path = "resource_rows/tests.rs"]
mod tests;
