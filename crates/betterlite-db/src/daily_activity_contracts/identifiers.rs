use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DailyActivityTable {
    #[serde(rename = "litellm_dailyuserspend")]
    User,
    #[serde(rename = "litellm_dailyteamspend")]
    Team,
    #[serde(rename = "litellm_dailytagspend")]
    Tag,
    #[serde(rename = "litellm_dailyorganizationspend")]
    Organization,
    #[serde(rename = "litellm_dailyenduserspend")]
    Customer,
    #[serde(rename = "litellm_dailyagentspend")]
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityIdField {
    UserId,
    TeamId,
    Tag,
    OrganizationId,
    EndUserId,
    AgentId,
}

impl EntityIdField {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::UserId => "user_id",
            Self::TeamId => "team_id",
            Self::Tag => "tag",
            Self::OrganizationId => "organization_id",
            Self::EndUserId => "end_user_id",
            Self::AgentId => "agent_id",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DailyActivityMetadata {
    pub table_name: &'static str,
    pub entity_field: EntityIdField,
    pub relation: &'static str,
}

impl DailyActivityTable {
    #[must_use]
    pub const fn metadata(self) -> DailyActivityMetadata {
        match self {
            Self::User => DailyActivityMetadata {
                table_name: "litellm_dailyuserspend",
                entity_field: EntityIdField::UserId,
                relation: "LiteLLM_DailyUserSpend",
            },
            Self::Team => DailyActivityMetadata {
                table_name: "litellm_dailyteamspend",
                entity_field: EntityIdField::TeamId,
                relation: "LiteLLM_DailyTeamSpend",
            },
            Self::Tag => DailyActivityMetadata {
                table_name: "litellm_dailytagspend",
                entity_field: EntityIdField::Tag,
                relation: "LiteLLM_DailyTagSpend",
            },
            Self::Organization => DailyActivityMetadata {
                table_name: "litellm_dailyorganizationspend",
                entity_field: EntityIdField::OrganizationId,
                relation: "LiteLLM_DailyOrganizationSpend",
            },
            Self::Customer => DailyActivityMetadata {
                table_name: "litellm_dailyenduserspend",
                entity_field: EntityIdField::EndUserId,
                relation: "LiteLLM_DailyEndUserSpend",
            },
            Self::Agent => DailyActivityMetadata {
                table_name: "litellm_dailyagentspend",
                entity_field: EntityIdField::AgentId,
                relation: "LiteLLM_DailyAgentSpend",
            },
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DailyActivityError {
    #[error("Invalid entity_id_field {field:?} for {table:?}")]
    EntityField {
        field: String,
        table: DailyActivityTable,
    },
}
