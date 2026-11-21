// the log level each LiteLLM call in this slice writes its diagnostic at
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Debug,
    Warning,
}

impl DiagnosticLevel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Warning => "warning",
        }
    }
}

// one record per diagnostic LiteLLM logs, in the order it logs them; the caller decides where they go
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideDiagnostic {
    // a team alias pointing at model_name_{team_id}_{uuid} whose deployment is gone
    StaleAliasMissingDeployment {
        model: String,
        team_id: String,
        aliased_target: String,
    },
    // the stale target still exists, but the team has sibling deployments for the requested name
    StaleAliasSiblingDeployment {
        model: String,
        team_id: String,
    },
    // _resolve_credential_from_model_config found an entry with no litellm_credentials
    ModelEntryWithoutCredentials {
        model: String,
    },
    // model_config names a credential CredentialAccessor cannot resolve
    CredentialNotFound {
        credential: String,
    },
    // the credential's values were copied onto the request
    CredentialApplied {
        credential: String,
        model: String,
    },
}

impl OverrideDiagnostic {
    #[must_use]
    pub const fn level(&self) -> DiagnosticLevel {
        match self {
            Self::StaleAliasMissingDeployment { .. }
            | Self::StaleAliasSiblingDeployment { .. }
            | Self::CredentialNotFound { .. } => DiagnosticLevel::Warning,
            Self::ModelEntryWithoutCredentials { .. } | Self::CredentialApplied { .. } => {
                DiagnosticLevel::Debug
            }
        }
    }

    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::StaleAliasMissingDeployment { .. } => "stale_alias_missing_deployment",
            Self::StaleAliasSiblingDeployment { .. } => "stale_alias_sibling_deployment",
            Self::ModelEntryWithoutCredentials { .. } => "model_entry_without_credentials",
            Self::CredentialNotFound { .. } => "credential_not_found",
            Self::CredentialApplied { .. } => "credential_applied",
        }
    }
}
