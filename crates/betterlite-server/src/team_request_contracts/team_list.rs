use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::archived::LiteLLMDeletedTeamTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use serde::{Deserialize, Serialize};

use super::TeamContractError;
use super::shared::{decode_request, object};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeamListItem {
    #[serde(flatten)]
    pub team: LiteLLMTeamTable,
    pub members_count: i64,
    pub keys_count: i64,
    pub access_group_models: Option<Vec<String>>,
    pub access_group_mcp_server_ids: Option<Vec<String>>,
    pub access_group_agent_ids: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct TeamListCounts {
    #[serde(default)]
    members_count: i64,
    #[serde(default)]
    keys_count: i64,
    access_group_models: Option<Vec<String>>,
    access_group_mcp_server_ids: Option<Vec<String>>,
    access_group_agent_ids: Option<Vec<String>>,
}

impl TeamListItem {
    pub fn from_value(value: &JsonData) -> Result<Self, TeamContractError> {
        let team = LiteLLMTeamTable::try_from(value.clone())?;
        let counts: TeamListCounts = decode_request(value)?;
        Ok(Self {
            team,
            members_count: counts.members_count,
            keys_count: counts.keys_count,
            access_group_models: counts.access_group_models,
            access_group_mcp_server_ids: counts.access_group_mcp_server_ids,
            access_group_agent_ids: counts.access_group_agent_ids,
        })
    }
}

const ENRICHED_KEYS: [&str; 5] = [
    "members_count",
    "keys_count",
    "access_group_models",
    "access_group_mcp_server_ids",
    "access_group_agent_ids",
];

const DELETION_KEYS: [&str; 5] = [
    "id",
    "deleted_at",
    "deleted_by",
    "deleted_by_api_key",
    "litellm_changed_by",
];

// The declared union: Python's smart union validates every member and keeps the candidates whose
// fields all match, scoring each by how many of the row's keys it declares; the enriched member,
// the bare table and the audit row win ties in that order, so a row reads as deleted only when it
// names more deletion audit fields than enriched ones and is a valid audit row.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TeamListEntry {
    Enriched(TeamListItem),
    Live(LiteLLMTeamTable),
    Deleted(LiteLLMDeletedTeamTable),
}

impl TeamListEntry {
    pub fn from_value(value: &JsonData) -> Result<Self, TeamContractError> {
        let fields = object(value)?;
        let enriched_matched = ENRICHED_KEYS
            .iter()
            .filter(|key| fields.contains_key(**key))
            .count();
        let deleted_matched = DELETION_KEYS
            .iter()
            .filter(|key| fields.contains_key(**key))
            .count();
        match (
            TeamListItem::from_value(value),
            LiteLLMDeletedTeamTable::try_from(value.clone()),
        ) {
            (Ok(item), Ok(row)) => {
                if deleted_matched > enriched_matched {
                    Ok(Self::Deleted(row))
                } else {
                    Ok(Self::Enriched(item))
                }
            }
            (Ok(item), Err(_)) => Ok(Self::Enriched(item)),
            (Err(_), Ok(row)) if deleted_matched > 0 => Ok(Self::Deleted(row)),
            (Err(_), _) => match LiteLLMTeamTable::try_from(value.clone()) {
                Ok(team) => Ok(Self::Live(team)),
                Err(error) => Err(error.into()),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TeamListResponse {
    pub teams: Vec<TeamListEntry>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
    pub total_pages: i64,
}

#[derive(Deserialize)]
struct TeamListInput {
    teams: Vec<JsonData>,
    total: i64,
    page: i64,
    page_size: i64,
    total_pages: i64,
}

impl TeamListResponse {
    pub fn from_value(value: &JsonData) -> Result<Self, TeamContractError> {
        let input: TeamListInput = decode_request(value)?;
        let teams = input
            .teams
            .iter()
            .map(TeamListEntry::from_value)
            .collect::<Result<Vec<TeamListEntry>, TeamContractError>>()?;
        Ok(Self {
            teams,
            total: input.total,
            page: input.page,
            page_size: input.page_size,
            total_pages: input.total_pages,
        })
    }
}
