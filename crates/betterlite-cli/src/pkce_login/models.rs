use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliAuthContract {
    pub contract_version: u8,
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: String,
    pub revocation_endpoint: String,
    pub resource: String,
    #[serde(rename = "code_challenge_methods_supported")]
    pub supported_methods: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PkceCredential {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub client_id: String,
    pub token_endpoint: String,
    pub revocation_endpoint: String,
    pub resource: String,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkceFailure {
    pub reason: String,
}

impl PkceFailure {
    pub(crate) fn new(reason: &str) -> Self {
        let message = String::from(reason);
        Self { reason: message }
    }

    pub(crate) const fn new_owned(reason: String) -> Self {
        if reason.is_empty() {
            return Self { reason };
        }
        Self { reason }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevocationUnavailable {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackCode {
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackDenied {
    pub error: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackOutcome {
    Code(CallbackCode),
    Denied(CallbackDenied),
}
