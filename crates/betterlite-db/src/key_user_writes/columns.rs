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

#[derive(Debug, Clone, Copy)]
pub struct UserColumnDef {
    pub name: &'static str,
    pub sql_type: TokenSqlType,
}

impl UserColumn {
    #[must_use]
    pub const fn definition(self) -> UserColumnDef {
        match self {
            Self::UserId => UserColumnDef {
                name: "user_id",
                sql_type: TokenSqlType::Text,
            },
            Self::UserAlias => UserColumnDef {
                name: "user_alias",
                sql_type: TokenSqlType::Text,
            },
            Self::TeamId => UserColumnDef {
                name: "team_id",
                sql_type: TokenSqlType::Text,
            },
            Self::SsoUserId => UserColumnDef {
                name: "sso_user_id",
                sql_type: TokenSqlType::Text,
            },
            Self::OrganizationId => UserColumnDef {
                name: "organization_id",
                sql_type: TokenSqlType::Text,
            },
            Self::ObjectPermissionId => UserColumnDef {
                name: "object_permission_id",
                sql_type: TokenSqlType::Text,
            },
            Self::Password => UserColumnDef {
                name: "password",
                sql_type: TokenSqlType::Text,
            },
            Self::PasswordResetRequired => UserColumnDef {
                name: "password_reset_required",
                sql_type: TokenSqlType::Boolean,
            },
            Self::LastBreachCheckAt => UserColumnDef {
                name: "last_breach_check_at",
                sql_type: TokenSqlType::Timestamp,
            },
            Self::Teams => UserColumnDef {
                name: "teams",
                sql_type: TokenSqlType::TextArray,
            },
            Self::UserRole => UserColumnDef {
                name: "user_role",
                sql_type: TokenSqlType::Text,
            },
            Self::MaxBudget => UserColumnDef {
                name: "max_budget",
                sql_type: TokenSqlType::DoublePrecision,
            },
            Self::Spend => UserColumnDef {
                name: "spend",
                sql_type: TokenSqlType::DoublePrecision,
            },
            Self::UserEmail => UserColumnDef {
                name: "user_email",
                sql_type: TokenSqlType::Text,
            },
            Self::Models => UserColumnDef {
                name: "models",
                sql_type: TokenSqlType::TextArray,
            },
            Self::Metadata => UserColumnDef {
                name: "metadata",
                sql_type: TokenSqlType::Json,
            },
            Self::MaxParallelRequests => UserColumnDef {
                name: "max_parallel_requests",
                sql_type: TokenSqlType::BigInt,
            },
            Self::TpmLimit => UserColumnDef {
                name: "tpm_limit",
                sql_type: TokenSqlType::BigInt,
            },
            Self::RpmLimit => UserColumnDef {
                name: "rpm_limit",
                sql_type: TokenSqlType::BigInt,
            },
            Self::BudgetDuration => UserColumnDef {
                name: "budget_duration",
                sql_type: TokenSqlType::Text,
            },
            Self::BudgetResetAt => UserColumnDef {
                name: "budget_reset_at",
                sql_type: TokenSqlType::Timestamp,
            },
            Self::AllowedCacheControls => UserColumnDef {
                name: "allowed_cache_controls",
                sql_type: TokenSqlType::TextArray,
            },
            Self::Policies => UserColumnDef {
                name: "policies",
                sql_type: TokenSqlType::TextArray,
            },
            Self::ModelSpend => UserColumnDef {
                name: "model_spend",
                sql_type: TokenSqlType::Json,
            },
            Self::ModelMaxBudget => UserColumnDef {
                name: "model_max_budget",
                sql_type: TokenSqlType::Json,
            },
            Self::CreatedAt => UserColumnDef {
                name: "created_at",
                sql_type: TokenSqlType::Timestamp,
            },
            Self::UpdatedAt => UserColumnDef {
                name: "updated_at",
                sql_type: TokenSqlType::Timestamp,
            },
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

    #[must_use]
    pub(super) fn bindings(
        &self,
    ) -> impl Iterator<Item = (&'static str, TokenSqlType, &TokenValue)> {
        self.0.iter().map(|(column, value)| {
            let definition = column.definition();
            (definition.name, definition.sql_type, value)
        })
    }
}
