use std::collections::BTreeMap;
use std::str::Utf8Error;
use std::str::from_utf8;

use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_redis::config_sync::table_actions::{ConfigSyncTableActions, TableActions as _};
use serde::{Deserialize, Serialize};

use crate::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableBinding, TableError,
};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::DbError;

use super::table::{DocumentData, ResourceActions, ResourceBinding};

pub const MAX_CONFIG_VALUE_BYTES: usize = 0x0010_0000;
pub const MAX_CONFIG_VALUE_DEPTH: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum ConfigDecodeError {
    #[error("config value is {len} bytes, over the {MAX_CONFIG_VALUE_BYTES} byte limit")]
    TooLarge { len: usize },
    #[error("config value is not valid UTF-8")]
    InvalidUtf8(#[source] Utf8Error),
    #[error("config value exceeds JSON depth {MAX_CONFIG_VALUE_DEPTH}")]
    TooDeep,
    #[error("config value is invalid JSON")]
    Json(#[source] serde_json::Error),
}

// A plain serde map keeps the last of duplicate keys, as json.loads does.
#[derive(Deserialize)]
#[serde(untagged)]
enum ParsedValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl From<ParsedValue> for JsonData {
    fn from(value: ParsedValue) -> Self {
        match value {
            ParsedValue::Null => Self::Null,
            ParsedValue::Bool(flag) => Self::Bool(flag),
            ParsedValue::Number(number) => Self::Number(number),
            ParsedValue::String(text) => Self::String(text),
            ParsedValue::Array(items) => Self::Array(items.into_iter().map(Self::from).collect()),
            ParsedValue::Object(fields) => Self::Object(
                fields
                    .into_iter()
                    .map(|(name, field)| (name, Self::from(field)))
                    .collect(),
            ),
        }
    }
}

#[derive(Clone, Copy)]
enum ScanState {
    Structural,
    Quoted,
    Escaped,
}

fn check_depth(text: &str) -> Result<(), ConfigDecodeError> {
    let mut state = ScanState::Structural;
    let mut depth = 0_usize;
    for byte in text.bytes() {
        match (state, byte) {
            (ScanState::Structural, b'{' | b'[') => {
                depth = depth.saturating_add(1);
                if depth > MAX_CONFIG_VALUE_DEPTH {
                    return Err(ConfigDecodeError::TooDeep);
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

pub fn decode_config_json(bytes: &[u8]) -> Result<JsonData, ConfigDecodeError> {
    if bytes.len() > MAX_CONFIG_VALUE_BYTES {
        return Err(ConfigDecodeError::TooLarge { len: bytes.len() });
    }
    let text = from_utf8(bytes).map_err(ConfigDecodeError::InvalidUtf8)?;
    check_depth(text)?;
    let parsed: ParsedValue = serde_json::from_str(text).map_err(ConfigDecodeError::Json)?;
    Ok(JsonData::from(parsed))
}

pub struct ConfigBinding;

impl TableBinding for ConfigBinding {
    const NAME: &'static str = "litellm_config";
    const TABLE: &'static str = "LiteLLM_Config";
}

impl ResourceBinding for ConfigBinding {
    const KEY: &'static str = "param_name";
    const ID: Option<&'static str> = None;
    const UPDATED_AT: Option<&'static str> = None;
    type Create = ConfigWrite;
    type Update = ConfigUpdate;
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigWrite {
    pub param_name: String,
    pub param_value: JsonData,
}

impl DocumentData for ConfigWrite {}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ConfigUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_value: Option<JsonData>,
}

impl DocumentData for ConfigUpdate {}

#[derive(Debug, Clone, Deserialize)]
pub struct ConfigStoredRow {
    pub param_name: String,
    pub param_value: JsonData,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigParam {
    pub param_name: String,
    pub param_value: JsonData,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Json(#[from] ConfigDecodeError),
}

pub struct ConfigRepository<'db> {
    base: PrismaTableRepository<'db, ConfigBinding>,
}

impl<'db> ConfigRepository<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, use_writer: bool) -> Self {
        let base =
            PrismaTableRepository::new(client, redis_config_change_publisher(None), ConfigBinding);
        Self {
            base: if use_writer { base.with_writer() } else { base },
        }
    }

    // ast-grep-ignore: no-one-line-functions - LiteLLM's ConfigRepository prisma_client property
    pub const fn prisma_client(&self) -> Result<PrismaClient<'db>, DbError> {
        self.base.prisma_client()
    }

    // ast-grep-ignore: no-one-line-functions - LiteLLM's table property builds the same actions
    pub fn table(
        &self,
    ) -> Result<ConfigSyncTableActions<ResourceActions<'db, ConfigBinding>>, DbError> {
        self.base.table(ResourceActions::new)
    }

    pub async fn get_param(&self, param_name: &str) -> Result<Option<ConfigParam>, ConfigError> {
        let Some(row) = self
            .table()?
            .as_ref()
            .find_unique::<ConfigStoredRow>(param_name)
            .await?
        else {
            return Ok(None);
        };
        let param_value = match row.param_value {
            JsonData::String(raw) => decode_config_json(raw.as_bytes())?,
            other => other,
        };
        Ok(Some(ConfigParam {
            param_name: param_name.to_owned(),
            param_value,
        }))
    }

    pub async fn set_param(
        &self,
        param_name: &str,
        param_value: JsonData,
    ) -> Result<ConfigParam, ConfigError> {
        let value_json = match &param_value {
            JsonData::String(text) => text.clone(),
            other => serde_json::to_string(other).map_err(TableError::Row)?,
        };
        let write = ConfigWrite {
            param_name: param_name.to_owned(),
            param_value: JsonData::String(value_json),
        };
        self.table()?.upsert(param_name.to_owned(), write).await?;
        Ok(ConfigParam {
            param_name: param_name.to_owned(),
            param_value,
        })
    }
}
