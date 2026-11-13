use serde::{Deserialize, Serialize};

use super::{MCPOAuthIdentityBinding, ServerRecordError};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BindingMode {
    #[default]
    Disabled,
    Audit,
    Enforce,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallerField {
    #[default]
    UserEmail,
    UserId,
}

#[derive(Deserialize)]
pub struct RawIdentityBinding {
    #[serde(default)]
    pub mode: BindingMode,
    pub issuer: String,
    pub jwks_url: Option<String>,
    pub audiences: Vec<String>,
    #[serde(default = "default_principal_claim")]
    pub principal_claim: String,
    #[serde(default)]
    pub caller_field: CallerField,
    #[serde(default = "default_require_email_verified")]
    pub require_email_verified: bool,
}

fn default_principal_claim() -> String {
    "email".to_owned()
}

const fn default_require_email_verified() -> bool {
    true
}

impl TryFrom<RawIdentityBinding> for MCPOAuthIdentityBinding {
    type Error = ServerRecordError;

    fn try_from(raw: RawIdentityBinding) -> Result<Self, Self::Error> {
        if raw.audiences.is_empty() {
            return Err(ServerRecordError::Binding(
                "audiences must not be empty".to_owned(),
            ));
        }
        Ok(Self {
            mode: raw.mode,
            issuer: raw.issuer,
            jwks_url: raw.jwks_url,
            audiences: raw.audiences,
            principal_claim: raw.principal_claim,
            caller_field: raw.caller_field,
            require_email_verified: raw.require_email_verified,
        })
    }
}
