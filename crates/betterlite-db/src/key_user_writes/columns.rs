use std::collections::BTreeMap;

use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenSqlType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UserColumn {
    UserId,
    UserAlias,
    TeamId,
    SsoUserId,
    OrganizationId,
    ObjectPermissionId,
    Password,
    PasswordResetRequired,
    LastBreachCheckAt,
    Teams,
    UserRole,
    MaxBudget,
    Spend,
    UserEmail,
    Models,
    Metadata,
    MaxParallelRequests,
    TpmLimit,
    RpmLimit,
    BudgetDuration,
    BudgetResetAt,
    AllowedCacheControls,
    Policies,
    ModelSpend,
    ModelMaxBudget,
    CreatedAt,
    UpdatedAt,
}

impl UserColumn {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::UserId => "user_id",
            Self::UserAlias => "user_alias",
            Self::TeamId => "team_id",
            Self::SsoUserId => "sso_user_id",
            Self::OrganizationId => "organization_id",
            Self::ObjectPermissionId => "object_permission_id",
            Self::Password => "password",
            Self::PasswordResetRequired => "password_reset_required",
            Self::LastBreachCheckAt => "last_breach_check_at",
            Self::Teams => "teams",
            Self::UserRole => "user_role",
            Self::MaxBudget => "max_budget",
            Self::Spend => "spend",
            Self::UserEmail => "user_email",
            Self::Models => "models",
            Self::Metadata => "metadata",
            Self::MaxParallelRequests => "max_parallel_requests",
            Self::TpmLimit => "tpm_limit",
            Self::RpmLimit => "rpm_limit",
            Self::BudgetDuration => "budget_duration",
            Self::BudgetResetAt => "budget_reset_at",
            Self::AllowedCacheControls => "allowed_cache_controls",
            Self::Policies => "policies",
            Self::ModelSpend => "model_spend",
            Self::ModelMaxBudget => "model_max_budget",
            Self::CreatedAt => "created_at",
            Self::UpdatedAt => "updated_at",
        }
    }

    #[must_use]
    pub const fn sql_type(self) -> TokenSqlType {
        match self {
            Self::UserId
            | Self::UserAlias
            | Self::TeamId
            | Self::SsoUserId
            | Self::OrganizationId
            | Self::ObjectPermissionId
            | Self::Password
            | Self::UserRole
            | Self::UserEmail
            | Self::BudgetDuration => TokenSqlType::Text,
            Self::PasswordResetRequired => TokenSqlType::Boolean,
            Self::LastBreachCheckAt | Self::BudgetResetAt | Self::CreatedAt | Self::UpdatedAt => {
                TokenSqlType::Timestamp
            }
            Self::Teams | Self::Models | Self::AllowedCacheControls | Self::Policies => {
                TokenSqlType::TextArray
            }
            Self::Metadata | Self::ModelSpend | Self::ModelMaxBudget => TokenSqlType::Json,
            Self::MaxBudget | Self::Spend => TokenSqlType::DoublePrecision,
            Self::MaxParallelRequests | Self::TpmLimit | Self::RpmLimit => TokenSqlType::BigInt,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserWrite(pub BTreeMap<UserColumn, TokenValue>);

impl UserWrite {
    #[must_use]
    pub fn text(&self, column: UserColumn) -> Option<&str> {
        match self.0.get(&column) {
            Some(TokenValue::Text(value)) => Some(value),
            _ => None,
        }
    }
}
