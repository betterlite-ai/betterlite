macro_rules! token_columns {
    ($($variant:ident => $name:literal : $sql:ident),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        pub enum TokenColumn {
            $($variant),*
        }

        impl TokenColumn {
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),*
                }
            }

            #[must_use]
            pub const fn sql_type(self) -> TokenSqlType {
                match self {
                    $(Self::$variant => TokenSqlType::$sql),*
                }
            }
        }
    };
}

token_columns! {
    Token => "token" : Text,
    KeyName => "key_name" : Text,
    KeyAlias => "key_alias" : Text,
    SoftBudgetCooldown => "soft_budget_cooldown" : Boolean,
    Spend => "spend" : DoublePrecision,
    TotalSpend => "total_spend" : DoublePrecision,
    Expires => "expires" : Timestamp,
    Models => "models" : TextArray,
    Aliases => "aliases" : Json,
    Config => "config" : Json,
    RouterSettings => "router_settings" : Json,
    UserId => "user_id" : Text,
    TeamId => "team_id" : Text,
    AgentId => "agent_id" : Text,
    ProjectId => "project_id" : Text,
    Permissions => "permissions" : Json,
    MaxParallelRequests => "max_parallel_requests" : BigInt,
    Metadata => "metadata" : Json,
    Blocked => "blocked" : Boolean,
    TpmLimit => "tpm_limit" : BigInt,
    RpmLimit => "rpm_limit" : BigInt,
    TpdLimit => "tpd_limit" : BigInt,
    MaxBudget => "max_budget" : DoublePrecision,
    BudgetDuration => "budget_duration" : Text,
    BudgetResetAt => "budget_reset_at" : Timestamp,
    AllowedCacheControls => "allowed_cache_controls" : TextArray,
    AllowedRoutes => "allowed_routes" : TextArray,
    KeyType => "key_type" : Text,
    Policies => "policies" : TextArray,
    AccessGroupIds => "access_group_ids" : TextArray,
    ModelSpend => "model_spend" : Json,
    ModelMaxBudget => "model_max_budget" : Json,
    BudgetFallbacks => "budget_fallbacks" : Json,
    BudgetId => "budget_id" : Text,
    OrganizationId => "organization_id" : Text,
    ObjectPermissionId => "object_permission_id" : Text,
    CreatedAt => "created_at" : Timestamp,
    CreatedBy => "created_by" : Text,
    UpdatedAt => "updated_at" : Timestamp,
    UpdatedBy => "updated_by" : Text,
    SettingsUpdatedAt => "settings_updated_at" : Timestamp,
    LastActive => "last_active" : Timestamp,
    RotationCount => "rotation_count" : BigInt,
    AutoRotate => "auto_rotate" : Boolean,
    RotationInterval => "rotation_interval" : Text,
    LastRotationAt => "last_rotation_at" : Timestamp,
    KeyRotationAt => "key_rotation_at" : Timestamp,
    BudgetLimits => "budget_limits" : Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSqlType {
    Text,
    Boolean,
    BigInt,
    DoublePrecision,
    Json,
    Timestamp,
    TextArray,
}

impl TokenSqlType {
    pub(super) const fn cast(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Boolean => "boolean",
            Self::BigInt => "bigint",
            Self::DoublePrecision => "double precision",
            Self::Json => "jsonb",
            Self::Timestamp => "timestamp",
            Self::TextArray => "text[]",
        }
    }
}
