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
        let (name, sql_type) = match self {
            Self::UserId => ("user_id", TokenSqlType::Text),
            Self::UserAlias => ("user_alias", TokenSqlType::Text),
            Self::TeamId => ("team_id", TokenSqlType::Text),
            Self::SsoUserId => ("sso_user_id", TokenSqlType::Text),
            Self::OrganizationId => ("organization_id", TokenSqlType::Text),
            Self::ObjectPermissionId => ("object_permission_id", TokenSqlType::Text),
            Self::Password => ("password", TokenSqlType::Text),
            Self::PasswordResetRequired => ("password_reset_required", TokenSqlType::Boolean),
            Self::LastBreachCheckAt => ("last_breach_check_at", TokenSqlType::Timestamp),
            Self::Teams => ("teams", TokenSqlType::TextArray),
            Self::UserRole => ("user_role", TokenSqlType::Text),
            Self::MaxBudget => ("max_budget", TokenSqlType::DoublePrecision),
            Self::Spend => ("spend", TokenSqlType::DoublePrecision),
            Self::UserEmail => ("user_email", TokenSqlType::Text),
            Self::Models => ("models", TokenSqlType::TextArray),
            Self::Metadata => ("metadata", TokenSqlType::Json),
            Self::MaxParallelRequests => ("max_parallel_requests", TokenSqlType::BigInt),
            Self::TpmLimit => ("tpm_limit", TokenSqlType::BigInt),
            Self::RpmLimit => ("rpm_limit", TokenSqlType::BigInt),
            Self::BudgetDuration => ("budget_duration", TokenSqlType::Text),
            Self::BudgetResetAt => ("budget_reset_at", TokenSqlType::Timestamp),
            Self::AllowedCacheControls => ("allowed_cache_controls", TokenSqlType::TextArray),
            Self::Policies => ("policies", TokenSqlType::TextArray),
            Self::ModelSpend => ("model_spend", TokenSqlType::Json),
            Self::ModelMaxBudget => ("model_max_budget", TokenSqlType::Json),
            Self::CreatedAt => ("created_at", TokenSqlType::Timestamp),
            Self::UpdatedAt => ("updated_at", TokenSqlType::Timestamp),
        };
        UserColumnDef { name, sql_type }
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

    pub(super) fn bindings(
        &self,
    ) -> impl Iterator<Item = (&'static str, TokenSqlType, &TokenValue)> {
        self.0.iter().map(|(column, value)| {
            let definition = column.definition();
            (definition.name, definition.sql_type, value)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserColumnOp {
    Set(TokenValue),
    Increment(TokenValue),
    Decrement(TokenValue),
    Multiply(TokenValue),
    Divide(TokenValue),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserUpdateOps(pub BTreeMap<UserColumn, UserColumnOp>);

impl From<&UserWrite> for UserUpdateOps {
    fn from(fields: &UserWrite) -> Self {
        Self(
            fields
                .0
                .iter()
                .map(|(column, value)| (*column, UserColumnOp::Set(value.clone())))
                .collect(),
        )
    }
}
