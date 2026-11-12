use std::sync::Arc;
use std::time::SystemTime;

use secrecy::SecretString;

use super::state::TokenState;

// the credential properties LiteLLM keeps on the credentials object it caches; the token is secret from the start
#[derive(Debug, Clone)]
pub struct CredentialSnapshot {
    pub token: Option<SecretString>,
    pub state: Option<TokenState>,
    pub expires_at: Option<SystemTime>,
    pub expired: Option<bool>,
    pub valid: Option<bool>,
    pub quota_project_id: Option<String>,
    pub project_id: Option<String>,
}

// LiteLLM caches either a (credentials, project) pair or, from older releases, the credentials alone
#[derive(Debug, Clone)]
pub enum CacheRecord {
    Paired {
        credential: Arc<CredentialSnapshot>,
        project: Option<String>,
    },
    Legacy {
        credential: Arc<CredentialSnapshot>,
    },
}

impl CacheRecord {
    #[must_use]
    pub const fn credential(&self) -> &Arc<CredentialSnapshot> {
        match self {
            Self::Paired { credential, .. } | Self::Legacy { credential } => credential,
        }
    }

    #[must_use]
    pub fn with_credential(&self, credential: Arc<CredentialSnapshot>) -> Self {
        match self {
            Self::Paired { project, .. } => Self::Paired {
                credential,
                project: project.clone(),
            },
            Self::Legacy { .. } => Self::Legacy { credential },
        }
    }
}

// LiteLLM's _unpack_cached_credentials: absent is (None, None), and a legacy record recovers quota_project_id before project_id
#[must_use]
pub fn unpack_cached_credentials(
    record: Option<&CacheRecord>,
) -> (Option<&Arc<CredentialSnapshot>>, Option<&str>) {
    match record {
        None => (None, None),
        Some(CacheRecord::Paired {
            credential,
            project,
        }) => (Some(credential), project.as_deref()),
        Some(CacheRecord::Legacy { credential }) => (
            Some(credential),
            credential
                .quota_project_id
                .as_deref()
                .or(credential.project_id.as_deref()),
        ),
    }
}
