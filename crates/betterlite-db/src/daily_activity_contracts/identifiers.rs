use serde::{Deserialize, Serialize};
use thiserror::Error;
macro_rules! tables {
    ($($variant:ident => ($table:literal, $field:literal, $relation:literal)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum DailyActivityTable { $(#[serde(rename = $table)] $variant,)+ }
        impl DailyActivityTable {
            #[must_use]
            pub const fn metadata(self) -> DailyActivityMetadata {
                match self {
                    $(Self::$variant => DailyActivityMetadata {
                        table_name: $table, entity_field: $field, relation: $relation,
                    },)+
                }
            }
        }
    };
}
tables! {
    User => ("litellm_dailyuserspend", "user_id", "LiteLLM_DailyUserSpend"),
    Team => ("litellm_dailyteamspend", "team_id", "LiteLLM_DailyTeamSpend"),
    Tag => ("litellm_dailytagspend", "tag", "LiteLLM_DailyTagSpend"),
    Organization => ("litellm_dailyorganizationspend", "organization_id", "LiteLLM_DailyOrganizationSpend"),
    Customer => ("litellm_dailyenduserspend", "end_user_id", "LiteLLM_DailyEndUserSpend"),
    Agent => ("litellm_dailyagentspend", "agent_id", "LiteLLM_DailyAgentSpend"),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DailyActivityMetadata {
    pub table_name: &'static str,
    pub entity_field: &'static str,
    pub relation: &'static str,
}
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DailyActivityError {
    #[error("Invalid entity_id_field '{field}' for {table}")]
    EntityField { field: String, table: &'static str },
}
