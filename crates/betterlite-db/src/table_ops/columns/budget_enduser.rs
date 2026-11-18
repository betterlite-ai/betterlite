use crate::key_repository::table::columns::TokenSqlType;

use super::TableColumn;
use crate::table_ops::access::{
    BUDGET_ATTRIBUTE, BUDGET_TABLE, END_USER_ATTRIBUTE, END_USER_TABLE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BudgetColumn {
    BudgetId,
    MaxBudget,
    SoftBudget,
    MaxParallelRequests,
    TpmLimit,
    RpmLimit,
    TpdLimit,
    ModelMaxBudget,
    BudgetDuration,
    BudgetResetAt,
    CreatedAt,
    CreatedBy,
    UpdatedAt,
    UpdatedBy,
    AllowedModels,
    TempBudgetIncrease,
    TempBudgetExpiry,
}

impl TableColumn for BudgetColumn {
    const TABLE: &'static str = BUDGET_TABLE;
    const ATTRIBUTE: &'static str = BUDGET_ATTRIBUTE;
    const KEY: Self = Self::BudgetId;

    fn definition(self) -> (&'static str, TokenSqlType) {
        match self {
            Self::BudgetId => ("budget_id", TokenSqlType::Text),
            Self::MaxBudget => ("max_budget", TokenSqlType::DoublePrecision),
            Self::SoftBudget => ("soft_budget", TokenSqlType::DoublePrecision),
            Self::MaxParallelRequests => ("max_parallel_requests", TokenSqlType::BigInt),
            Self::TpmLimit => ("tpm_limit", TokenSqlType::BigInt),
            Self::RpmLimit => ("rpm_limit", TokenSqlType::BigInt),
            Self::TpdLimit => ("tpd_limit", TokenSqlType::BigInt),
            Self::ModelMaxBudget => ("model_max_budget", TokenSqlType::Json),
            Self::BudgetDuration => ("budget_duration", TokenSqlType::Text),
            Self::BudgetResetAt => ("budget_reset_at", TokenSqlType::Timestamp),
            Self::CreatedAt => ("created_at", TokenSqlType::Timestamp),
            Self::CreatedBy => ("created_by", TokenSqlType::Text),
            Self::UpdatedAt => ("updated_at", TokenSqlType::Timestamp),
            Self::UpdatedBy => ("updated_by", TokenSqlType::Text),
            Self::AllowedModels => ("allowed_models", TokenSqlType::TextArray),
            Self::TempBudgetIncrease => ("temp_budget_increase", TokenSqlType::DoublePrecision),
            Self::TempBudgetExpiry => ("temp_budget_expiry", TokenSqlType::Timestamp),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EndUserColumn {
    UserId,
    Alias,
    Spend,
    AllowedModelRegion,
    DefaultModel,
    BudgetId,
    Blocked,
    ObjectPermissionId,
}

impl TableColumn for EndUserColumn {
    const TABLE: &'static str = END_USER_TABLE;
    const ATTRIBUTE: &'static str = END_USER_ATTRIBUTE;
    const KEY: Self = Self::UserId;

    fn definition(self) -> (&'static str, TokenSqlType) {
        match self {
            Self::UserId => ("user_id", TokenSqlType::Text),
            Self::Alias => ("alias", TokenSqlType::Text),
            Self::Spend => ("spend", TokenSqlType::DoublePrecision),
            Self::AllowedModelRegion => ("allowed_model_region", TokenSqlType::Text),
            Self::DefaultModel => ("default_model", TokenSqlType::Text),
            Self::BudgetId => ("budget_id", TokenSqlType::Text),
            Self::Blocked => ("blocked", TokenSqlType::Boolean),
            Self::ObjectPermissionId => ("object_permission_id", TokenSqlType::Text),
        }
    }
}
