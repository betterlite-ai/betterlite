use rust_decimal::Decimal;

use super::ScalarFilters;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpendBounds {
    pub minimum_inclusive: Option<Decimal>,
    pub maximum_inclusive: Option<Decimal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqualityColumn {
    TeamId,
    User,
    ApiKey,
    Model,
    ModelId,
    ModelGroup,
    EndUser,
}

impl EqualityColumn {
    // ui_view_spend_logs inserts the equality keys into where_conditions in this order
    pub const WHERE_CONDITION_ORDER: [Self; 7] = [
        Self::TeamId,
        Self::ApiKey,
        Self::User,
        Self::Model,
        Self::ModelId,
        Self::ModelGroup,
        Self::EndUser,
    ];

    #[must_use]
    pub const fn sql_column(self) -> &'static str {
        match self {
            Self::TeamId => "team_id",
            Self::User => "\"user\"",
            Self::ApiKey => "api_key",
            Self::Model => "model",
            Self::ModelId => "model_id",
            Self::ModelGroup => "model_group",
            Self::EndUser => "end_user",
        }
    }

    #[must_use]
    pub fn value_in(self, filters: &ScalarFilters) -> Option<&str> {
        match self {
            Self::TeamId => filters.team_id.as_deref(),
            Self::User => filters.user_id.as_deref(),
            Self::ApiKey => filters.api_key.as_deref(),
            Self::Model => filters.model.as_deref(),
            Self::ModelId => filters.model_id.as_deref(),
            Self::ModelGroup => filters.model_group.as_deref(),
            Self::EndUser => filters.end_user.as_deref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EqualityFilter {
    pub column: EqualityColumn,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataOperator {
    Equals,
    Like,
}

impl MetadataOperator {
    #[must_use]
    pub const fn sql(self) -> &'static str {
        match self {
            Self::Equals => "=",
            Self::Like => "LIKE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataFilter {
    KeyAlias(String),
    ErrorCode(String),
    ErrorMessage(String),
}

impl MetadataFilter {
    #[must_use]
    pub const fn sql_column(&self) -> &'static str {
        match self {
            Self::KeyAlias(_) => "metadata->>'user_api_key_alias'",
            Self::ErrorCode(_) => "metadata->'error_information'->>'error_code'",
            Self::ErrorMessage(_) => "metadata->'error_information'->>'error_message'",
        }
    }

    #[must_use]
    pub const fn operator(&self) -> MetadataOperator {
        match self {
            Self::KeyAlias(_) | Self::ErrorMessage(_) => MetadataOperator::Like,
            Self::ErrorCode(_) => MetadataOperator::Equals,
        }
    }

    #[must_use]
    pub fn parameter(&self) -> String {
        match self {
            Self::KeyAlias(value) | Self::ErrorMessage(value) => format!("%{value}%"),
            Self::ErrorCode(value) => value.clone(),
        }
    }
}
