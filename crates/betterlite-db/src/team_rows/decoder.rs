use std::str::{Utf8Error, from_utf8};

use super::{
    aliases::LiteLLMModelTable,
    archived::{LiteLLMDeletedTeamTable, LiteLLMTeamTableCachedObj},
    member::{BudgetLimitEntry, Member, MemberBase},
    membership::LiteLLMTeamMembership,
    team::{LiteLLMTeamTable, TeamBase},
};
use crate::budget_permission_rows::JsonData;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const MAX_ROW_BYTES: usize = 0x0010_0000;

#[derive(Debug, thiserror::Error)]
pub enum TeamRowError {
    #[error("stored row is {len} bytes, over the 1048576 byte limit")]
    TooLarge { len: usize },
    #[error("stored row is not valid UTF-8")]
    InvalidUtf8(#[source] Utf8Error),
    #[error("stored row exceeds JSON depth 64")]
    TooDeep,
    #[error("stored row is invalid JSON: {0}")]
    Json(#[source] serde_json::Error),
    #[error("input needs to be a dictionary")]
    NotAnObject,
    #[error("Either user id or user email must be provided")]
    MissingIdentity,
    #[error("team_id is required")]
    MissingTeamId,
    #[error("Field {field} should be a valid dictionary")]
    Field {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "row", rename_all = "snake_case")]
pub enum TeamRow {
    MemberBase(MemberBase),
    Member(Member),
    BudgetLimit(BudgetLimitEntry),
    Model(Box<LiteLLMModelTable>),
    TeamBase(Box<TeamBase>),
    Team(Box<LiteLLMTeamTable>),
    Cached(Box<LiteLLMTeamTableCachedObj>),
    Deleted(Box<LiteLLMDeletedTeamTable>),
    Membership(Box<LiteLLMTeamMembership>),
}

pub fn decode_team_rows(bytes: &[u8]) -> Result<TeamRow, TeamRowError> {
    let value = parse_data(bytes)?;
    if let JsonData::Object(envelope) = &value
        && let (Some(JsonData::String(kind)), Some(row)) =
            (envelope.get("kind"), envelope.get("row"))
    {
        validate_nested(row, kind)?;
    }
    convert(&value)
}

pub fn parse_data(bytes: &[u8]) -> Result<JsonData, TeamRowError> {
    if bytes.len() > MAX_ROW_BYTES {
        return Err(TeamRowError::TooLarge { len: bytes.len() });
    }
    let text = from_utf8(bytes).map_err(TeamRowError::InvalidUtf8)?;
    check_depth(text)?;
    serde_json::from_str(text).map_err(TeamRowError::Json)
}

pub(super) fn convert<T>(value: &JsonData) -> Result<T, TeamRowError>
where
    T: DeserializeOwned,
{
    let text = serde_json::to_string(value).map_err(TeamRowError::Json)?;
    if text.len() > MAX_ROW_BYTES {
        return Err(TeamRowError::TooLarge { len: text.len() });
    }
    check_depth(&text)?;
    serde_json::from_str(&text).map_err(TeamRowError::Json)
}

fn check_depth(text: &str) -> Result<(), TeamRowError> {
    let mut quoted = false;
    let mut escaped = false;
    let mut depth = 0_u8;
    for byte in text.bytes() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' if quoted => escaped = true,
            b'"' => quoted = !quoted,
            b'[' | b'{' if !quoted => {
                depth = depth.saturating_add(1);
                if depth > 64 {
                    return Err(TeamRowError::TooDeep);
                }
            }
            b']' | b'}' if !quoted => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

fn validate_nested(row: &JsonData, kind: &str) -> Result<(), TeamRowError> {
    let JsonData::Object(fields) = row else {
        return Ok(());
    };
    let relation = match kind {
        "team" | "cached" | "deleted" => {
            let mut normalized = row.clone();
            super::validator::set_model_info(&mut normalized)?;
            let text = serde_json::to_string(&normalized).map_err(TeamRowError::Json)?;
            if text.len() > MAX_ROW_BYTES {
                return Err(TeamRowError::TooLarge { len: text.len() });
            }
            check_depth(&text)?;
            fields
                .get("litellm_model_table")
                .map(|model| (model, "model"))
        }
        "model" => fields.get("team").map(|team| (team, "team")),
        _ => None,
    };
    if let Some((nested, shape)) = relation {
        validate_nested(nested, shape)?;
    }
    Ok(())
}
