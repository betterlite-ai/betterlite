use super::tags::{DailyTagsError, TagsPayload, tags_from_payload};
use super::values::SpendColumnValue;
use serde::Deserialize;
use std::collections::BTreeMap;

pub type SpendRow = BTreeMap<String, SpendColumnValue>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DailySpendEntity {
    User,
    Team,
    Org,
    Tag,
    EndUser,
    Agent,
}

#[derive(Debug, Clone, Copy)]
pub struct DailySpendTable {
    pub(super) name: &'static str,
    pub(super) entity_id_column: &'static str,
    pub(super) carries_request_id: bool,
}

impl DailySpendEntity {
    #[must_use]
    pub const fn table(self) -> DailySpendTable {
        match self {
            Self::User => DailySpendTable {
                name: "LiteLLM_DailyUserSpend",
                entity_id_column: "user_id",
                carries_request_id: false,
            },
            Self::Team => DailySpendTable {
                name: "LiteLLM_DailyTeamSpend",
                entity_id_column: "team_id",
                carries_request_id: false,
            },
            Self::Org => DailySpendTable {
                name: "LiteLLM_DailyOrganizationSpend",
                entity_id_column: "organization_id",
                carries_request_id: false,
            },
            Self::Tag => DailySpendTable {
                name: "LiteLLM_DailyTagSpend",
                entity_id_column: "tag",
                carries_request_id: true,
            },
            Self::EndUser => DailySpendTable {
                name: "LiteLLM_DailyEndUserSpend",
                entity_id_column: "end_user_id",
                carries_request_id: false,
            },
            Self::Agent => DailySpendTable {
                name: "LiteLLM_DailyAgentSpend",
                entity_id_column: "agent_id",
                carries_request_id: false,
            },
        }
    }
}

// The handful of fields daily_spend_entity_ids reads out of the much larger request/log payload; callers project their own payload type into this one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct DailySpendEntityPayload {
    #[serde(default, with = "serde_with::rust::double_option")]
    pub user: Option<Option<String>>,
    pub team_id: Option<String>,
    pub organization_id: Option<String>,
    pub end_user: Option<String>,
    pub agent_id: Option<String>,
    pub request_tags: Option<TagsPayload>,
}

pub fn daily_spend_entity_ids(
    payload: &DailySpendEntityPayload,
    entity: DailySpendEntity,
) -> Result<Vec<Option<String>>, DailyTagsError> {
    match entity {
        // user is the one entity whose explicit null still rolls up, as a None id.
        DailySpendEntity::User => Ok(match &payload.user {
            None => Vec::new(),
            Some(None) => vec![None],
            Some(Some(value)) => vec![Some(value.clone())],
        }),
        DailySpendEntity::Tag => match &payload.request_tags {
            None => Ok(Vec::new()),
            Some(tags) => Ok(tags_from_payload(tags.clone())?
                .into_iter()
                .map(Some)
                .collect()),
        },
        DailySpendEntity::EndUser => Ok(scalar_ids(payload.end_user.as_ref(), true)),
        DailySpendEntity::Team => Ok(scalar_ids(payload.team_id.as_ref(), false)),
        DailySpendEntity::Org => Ok(scalar_ids(payload.organization_id.as_ref(), false)),
        DailySpendEntity::Agent => Ok(scalar_ids(payload.agent_id.as_ref(), false)),
    }
}

fn scalar_ids(value: Option<&String>, drop_empty: bool) -> Vec<Option<String>> {
    match value {
        Some(text) if !(drop_empty && text.is_empty()) => vec![Some(text.clone())],
        _ => Vec::new(),
    }
}
