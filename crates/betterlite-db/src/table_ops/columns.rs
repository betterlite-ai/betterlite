pub mod budget_enduser;
pub mod spend_notification;

use crate::key_repository::table::columns::{TokenColumn, TokenSqlType};
use crate::key_user_writes::columns::UserColumn;

use super::access::{
    CONFIG_ATTRIBUTE, CONFIG_TABLE, KEY_ATTRIBUTE, KEY_TABLE, TEAM_ATTRIBUTE, TEAM_TABLE,
    USER_ATTRIBUTE, USER_TABLE,
};

// a closed set of columns per table, so a payload column is named once and its SQL type travels with it
pub trait TableColumn: Copy + Ord {
    const TABLE: &'static str;
    const ATTRIBUTE: &'static str;
    const KEY: Self;
    const UPDATED_AT: Option<Self> = None;
    fn sql_definition(self) -> (&'static str, TokenSqlType);
}

impl TableColumn for TokenColumn {
    const TABLE: &'static str = KEY_TABLE;
    const ATTRIBUTE: &'static str = KEY_ATTRIBUTE;
    const KEY: Self = Self::Token;
    const UPDATED_AT: Option<Self> = Some(Self::UpdatedAt);

    fn sql_definition(self) -> (&'static str, TokenSqlType) {
        (self.name(), self.sql_type())
    }
}

impl TableColumn for UserColumn {
    const TABLE: &'static str = USER_TABLE;
    const ATTRIBUTE: &'static str = USER_ATTRIBUTE;
    const KEY: Self = Self::UserId;
    const UPDATED_AT: Option<Self> = Some(Self::UpdatedAt);

    fn sql_definition(self) -> (&'static str, TokenSqlType) {
        let named = Self::definition(self);
        (named.name, named.sql_type)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigColumn {
    ParamName,
    ParamValue,
    LastRunAt,
    ReloadRevision,
}

impl TableColumn for ConfigColumn {
    const TABLE: &'static str = CONFIG_TABLE;
    const ATTRIBUTE: &'static str = CONFIG_ATTRIBUTE;
    const KEY: Self = Self::ParamName;

    fn sql_definition(self) -> (&'static str, TokenSqlType) {
        match self {
            Self::ParamName => ("param_name", TokenSqlType::Text),
            Self::ParamValue => ("param_value", TokenSqlType::Json),
            Self::LastRunAt => ("last_run_at", TokenSqlType::Timestamp),
            Self::ReloadRevision => ("reload_revision", TokenSqlType::BigInt),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TeamColumn {
    TeamId,
    TeamAlias,
    OrganizationId,
    ObjectPermissionId,
    Admins,
    Members,
    MembersWithRoles,
    Metadata,
    MaxBudget,
    SoftBudget,
    Spend,
    Models,
    MaxParallelRequests,
    TpmLimit,
    RpmLimit,
    TpdLimit,
    BudgetDuration,
    BudgetResetAt,
    Blocked,
    CreatedAt,
    UpdatedAt,
    ModelSpend,
    ModelMaxBudget,
    RouterSettings,
    TeamMemberPermissions,
    AccessGroupIds,
    Policies,
    DefaultTeamMemberModels,
    BudgetLimits,
    ModelId,
    AllowTeamGuardrailConfig,
}

impl TableColumn for TeamColumn {
    const TABLE: &'static str = TEAM_TABLE;
    const ATTRIBUTE: &'static str = TEAM_ATTRIBUTE;
    const KEY: Self = Self::TeamId;
    const UPDATED_AT: Option<Self> = Some(Self::UpdatedAt);

    fn sql_definition(self) -> (&'static str, TokenSqlType) {
        match self {
            Self::TeamId => ("team_id", TokenSqlType::Text),
            Self::TeamAlias => ("team_alias", TokenSqlType::Text),
            Self::OrganizationId => ("organization_id", TokenSqlType::Text),
            Self::ObjectPermissionId => ("object_permission_id", TokenSqlType::Text),
            Self::Admins => ("admins", TokenSqlType::TextArray),
            Self::Members => ("members", TokenSqlType::TextArray),
            Self::MembersWithRoles => ("members_with_roles", TokenSqlType::Json),
            Self::Metadata => ("metadata", TokenSqlType::Json),
            Self::MaxBudget => ("max_budget", TokenSqlType::DoublePrecision),
            Self::SoftBudget => ("soft_budget", TokenSqlType::DoublePrecision),
            Self::Spend => ("spend", TokenSqlType::DoublePrecision),
            Self::Models => ("models", TokenSqlType::TextArray),
            Self::MaxParallelRequests => ("max_parallel_requests", TokenSqlType::BigInt),
            Self::TpmLimit => ("tpm_limit", TokenSqlType::BigInt),
            Self::RpmLimit => ("rpm_limit", TokenSqlType::BigInt),
            Self::TpdLimit => ("tpd_limit", TokenSqlType::BigInt),
            Self::BudgetDuration => ("budget_duration", TokenSqlType::Text),
            Self::BudgetResetAt => ("budget_reset_at", TokenSqlType::Timestamp),
            Self::Blocked => ("blocked", TokenSqlType::Boolean),
            Self::CreatedAt => ("created_at", TokenSqlType::Timestamp),
            Self::UpdatedAt => ("updated_at", TokenSqlType::Timestamp),
            Self::ModelSpend => ("model_spend", TokenSqlType::Json),
            Self::ModelMaxBudget => ("model_max_budget", TokenSqlType::Json),
            Self::RouterSettings => ("router_settings", TokenSqlType::Json),
            Self::TeamMemberPermissions => ("team_member_permissions", TokenSqlType::TextArray),
            Self::AccessGroupIds => ("access_group_ids", TokenSqlType::TextArray),
            Self::Policies => ("policies", TokenSqlType::TextArray),
            Self::DefaultTeamMemberModels => {
                ("default_team_member_models", TokenSqlType::TextArray)
            }
            Self::BudgetLimits => ("budget_limits", TokenSqlType::Json),
            Self::ModelId => ("model_id", TokenSqlType::BigInt),
            Self::AllowTeamGuardrailConfig => {
                ("allow_team_guardrail_config", TokenSqlType::Boolean)
            }
        }
    }
}
