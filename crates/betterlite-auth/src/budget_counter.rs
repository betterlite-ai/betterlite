use rust_decimal::Decimal;
use std::time::SystemTime;
pub mod coercion;
mod tests;
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CounterEntityType {
    Key,
    Team,
    TeamMember,
    User,
    EndUser,
    Tag,
    ModelAccessGroup,
    Organization,
    Project,
}
impl CounterEntityType {
    #[must_use]
    pub fn from_litellm_name(name: &str) -> Option<Self> {
        match name {
            "Key" => Some(Self::Key),
            "Team" => Some(Self::Team),
            "TeamMember" => Some(Self::TeamMember),
            "User" => Some(Self::User),
            "EndUser" => Some(Self::EndUser),
            "Tag" => Some(Self::Tag),
            "Model access group" => Some(Self::ModelAccessGroup),
            "Organization" => Some(Self::Organization),
            "Project" => Some(Self::Project),
            _ => None,
        }
    }
    #[must_use]
    pub const fn wire_value(&self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Team => "team",
            Self::TeamMember => "team_member",
            Self::User => "user",
            Self::EndUser => "end_user",
            Self::Tag => "tag",
            Self::ModelAccessGroup => "model_access_group",
            Self::Organization => "organization",
            Self::Project => "project",
        }
    }
}
#[derive(Debug, Eq, PartialEq)]
pub struct BudgetCounter {
    pub counter_key: String,
    pub max_budget: Decimal,
    pub fallback_spend: Decimal,
    pub entity_type: CounterEntityType,
    pub entity_id: String,
    pub source_cache_key: Option<String>,
    pub spend_log_entity_id: Option<String>,
    pub window_duration: Option<String>,
    pub window_start: Option<SystemTime>,
}
