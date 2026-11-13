use crate::budget_permission_rows::JsonData;
use crate::team_rows::decoder::{TeamRowError, parse_data};
use crate::team_rows::member::Member;
use crate::team_rows::team::LiteLLMTeamTable;
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::BTreeMap;

// the stored columns LiteLLM's repository parses before the model validator sees the row
pub const JSON_ENCODED_TEAM_FIELDS: [&str; 6] = [
    "metadata",
    "model_spend",
    "model_max_budget",
    "router_settings",
    "budget_limits",
    "members_with_roles",
];

#[derive(Debug, thiserror::Error)]
pub enum TeamRecordError {
    #[error("a team record must be a JSON object")]
    NotAnObject,
    #[error("stored team field {field} cannot be decoded")]
    Field {
        field: String,
        #[source]
        source: TeamRowError,
    },
    #[error("stored team record cannot be decoded")]
    Record {
        #[source]
        source: TeamRowError,
    },
    #[error("members_with_roles is not a list of team members")]
    NotAList,
    #[error("stored team member cannot be decoded")]
    Member {
        #[source]
        source: TeamRowError,
    },
}

// the column order a row keeps is the order the repository decodes its JSON fields in
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct TeamRecord {
    pub fields: IndexMap<String, JsonData>,
}

impl TeamRecord {
    pub fn parse(bytes: &[u8]) -> Result<Option<Self>, TeamRecordError> {
        let value = parse_data(bytes).map_err(|source| TeamRecordError::Record { source })?;
        if matches!(value, JsonData::Null) {
            return Ok(None);
        }
        if !matches!(value, JsonData::Object(_)) {
            return Err(TeamRecordError::NotAnObject);
        }
        serde_json::from_slice(bytes)
            .map(Some)
            .map_err(|source| TeamRecordError::Record {
                source: TeamRowError::Json(source),
            })
    }
}

pub fn decode_team_record(
    record: Option<&TeamRecord>,
) -> Result<Option<LiteLLMTeamTable>, TeamRecordError> {
    let Some(found) = record else {
        return Ok(None);
    };
    convert_record(found).map(Some)
}

pub(super) fn convert_record(record: &TeamRecord) -> Result<LiteLLMTeamTable, TeamRecordError> {
    let mut decoded = BTreeMap::new();
    for (name, value) in &record.fields {
        let stored = JSON_ENCODED_TEAM_FIELDS.contains(&name.as_str());
        let converted = if let (true, JsonData::String(text)) = (stored, value) {
            parse_data(text.as_bytes()).map_err(|source| TeamRecordError::Field {
                field: name.clone(),
                source,
            })?
        } else {
            value.clone()
        };
        decoded.insert(name.clone(), converted);
    }
    LiteLLMTeamTable::try_from(JsonData::Object(decoded))
        .map_err(|source| TeamRecordError::Record { source })
}

pub fn decode_locked_roster(value: &JsonData) -> Result<Vec<Member>, TeamRecordError> {
    let parsed = if let JsonData::String(text) = value {
        parse_data(text.as_bytes()).map_err(|source| TeamRecordError::Record { source })?
    } else {
        value.clone()
    };
    if is_falsey(&parsed) {
        return Ok(Vec::new());
    }
    let JsonData::Array(entries) = parsed else {
        return Err(TeamRecordError::NotAList);
    };
    entries
        .into_iter()
        .map(|entry| Member::try_from(entry).map_err(|source| TeamRecordError::Member { source }))
        .collect()
}

fn is_falsey(value: &JsonData) -> bool {
    match value {
        JsonData::Null => true,
        JsonData::Bool(flag) => !flag,
        JsonData::Number(number) => number.as_f64().is_some_and(|amount| amount == 0.0_f64),
        JsonData::String(text) => text.is_empty(),
        JsonData::Array(entries) => entries.is_empty(),
        JsonData::Object(fields) => fields.is_empty(),
    }
}
