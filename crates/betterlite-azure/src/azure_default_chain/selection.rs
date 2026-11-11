use super::error::{DefaultChainError, invalid_filter_message};

const DEVELOPER_FILTERS: [&str; 6] = [
    "dev",
    "azureclicredential",
    "azuredeveloperclicredential",
    "azurepowershellcredential",
    "interactivebrowsercredential",
    "visualstudiocodecredential",
];

#[derive(Debug, Clone, Copy)]
pub enum Selection {
    All,
    Environment,
    Workload,
    Managed,
}

impl Selection {
    #[must_use]
    pub const fn includes_environment(self) -> bool {
        match self {
            Self::All | Self::Environment => true,
            Self::Workload | Self::Managed => false,
        }
    }

    #[must_use]
    pub const fn includes_workload(self) -> bool {
        match self {
            Self::All | Self::Workload => true,
            Self::Environment | Self::Managed => false,
        }
    }

    #[must_use]
    pub const fn includes_managed(self) -> bool {
        match self {
            Self::All | Self::Managed => true,
            Self::Environment | Self::Workload => false,
        }
    }
}

// AZURE_TOKEN_CREDENTIALS names a group or one link; anything else is the SDK's ValueError
pub fn selection(value: &str) -> Result<Selection, DefaultChainError> {
    match value {
        "" | "prod" => Ok(Selection::All),
        "environmentcredential" => Ok(Selection::Environment),
        "workloadidentitycredential" => Ok(Selection::Workload),
        "managedidentitycredential" => Ok(Selection::Managed),
        other if DEVELOPER_FILTERS.contains(&other) => Err(DefaultChainError::UnsupportedFilter {
            value: other.to_owned(),
        }),
        other => Err(DefaultChainError::InvalidFilter {
            message: invalid_filter_message(other),
        }),
    }
}
