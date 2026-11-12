use thiserror::Error;

pub const ENVIRONMENT_UNAVAILABLE: &str = "EnvironmentCredential authentication unavailable. Environment variables are not fully configured.\nVisit https://aka.ms/azsdk/python/identity/environmentcredential/troubleshoot to troubleshoot this issue.";
pub const WORKLOAD_UNAVAILABLE: &str = "WorkloadIdentityCredential authentication unavailable. The workload options are not fully configured. See the troubleshooting guide for more information: https://aka.ms/azsdk/python/identity/workloadidentitycredential/troubleshoot";
pub const TROUBLESHOOT_URL: &str =
    "https://aka.ms/azsdk/python/identity/defaultazurecredential/troubleshoot";
pub const EXHAUSTED: &str = "failed to retrieve a token from the included credentials.";

#[must_use]
pub fn workload_missing_message(missing: &[&str]) -> String {
    let listed: Vec<String> = missing.iter().map(|name| format!("'{name}'")).collect();
    format!(
        "{WORKLOAD_UNAVAILABLE}. Missing required arguments: {}.",
        listed.join(", ")
    )
}

#[must_use]
pub fn invalid_filter_message(value: &str) -> String {
    format!(
        "Invalid value for AZURE_TOKEN_CREDENTIALS: {value}. Valid values are: dev, prod, azureclicredential, azuredeveloperclicredential, azurepowershellcredential, environmentcredential, interactivebrowsercredential, managedidentitycredential, visualstudiocodecredential, workloadidentitycredential."
    )
}

#[derive(Debug, Error)]
pub enum DefaultChainError {
    #[error("environment variable {variable} is not set")]
    MissingEnvironment { variable: &'static str },
    #[error("{message}")]
    InvalidFilter { message: String },
    #[error(
        "AZURE_TOKEN_CREDENTIALS={value} selects only a developer credential this native build does not support"
    )]
    UnsupportedFilter { value: String },
    #[error("{message}")]
    Construction { message: String },
    #[error("{message}")]
    Credential { unavailable: bool, message: String },
    #[error("{message}")]
    Exhausted {
        chain: &'static str,
        message: String,
    },
    #[error("{} missing required arguments: {details}", credential)]
    MissingArguments {
        credential: &'static str,
        arguments: Vec<&'static str>,
        details: String,
    },
}

impl DefaultChainError {
    #[must_use]
    pub const fn category(&self) -> &'static str {
        match self {
            Self::MissingEnvironment { .. } => "missing_environment",
            Self::InvalidFilter { .. } => "invalid_value",
            Self::UnsupportedFilter { .. } => "unsupported",
            Self::Construction { .. } => "construction",
            Self::Credential { unavailable, .. } => {
                if *unavailable {
                    "unavailable"
                } else {
                    "client_authentication"
                }
            }
            Self::Exhausted { .. } => "client_authentication",
            Self::MissingArguments { .. } => "type_error",
        }
    }

    // the variable name and the argument list each belong to exactly one variant, so one match carries both
    #[must_use]
    pub fn detail(&self) -> (Option<&'static str>, Option<&[&'static str]>) {
        match self {
            Self::MissingEnvironment { variable } => (Some(*variable), None),
            Self::MissingArguments { arguments, .. } => (None, Some(arguments)),
            Self::InvalidFilter { .. }
            | Self::UnsupportedFilter { .. }
            | Self::Construction { .. }
            | Self::Credential { .. }
            | Self::Exhausted { .. } => (None, None),
        }
    }
}
