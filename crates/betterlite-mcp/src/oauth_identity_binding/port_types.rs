mod tests;

use http::StatusCode;
use secrecy::SecretString;
use serde::Serialize;

// LiteLLM's _RejectionCode literal, the two codes a rejected identity binding writes into its detail.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum RejectionCode {
    #[serde(rename = "oauth_principal_mismatch")]
    PrincipalMismatch,
    #[serde(rename = "oauth_identity_binding_failed")]
    BindingFailed,
}

impl RejectionCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PrincipalMismatch => "oauth_principal_mismatch",
            Self::BindingFailed => "oauth_identity_binding_failed",
        }
    }
}

// LiteLLM's RefreshOwnership alias: Proven is its own sealed envelope unwrapped, Presented is the caller's token, and its None case stays Option::None.
#[derive(Clone, Debug)]
pub enum RefreshOwnership {
    Proven,
    Presented(SecretString),
}

// B4715's refresh path builds the port's ownership from the stored refresh token, which the caller presented.
impl From<SecretString> for RefreshOwnership {
    fn from(refresh_token: SecretString) -> Self {
        Self::Presented(refresh_token)
    }
}

// LiteLLM's enforce-mode 403 as a value: its callers answer with status and detail, or read any denial as no token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingDenied {
    pub code: RejectionCode,
    pub description: String,
    pub server_id: String,
}

impl BindingDenied {
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self.code {
            RejectionCode::PrincipalMismatch | RejectionCode::BindingFailed => {
                StatusCode::FORBIDDEN
            }
        }
    }

    #[must_use]
    pub fn detail(&self) -> BindingDenialDetail<'_> {
        BindingDenialDetail {
            error: self.code,
            error_description: &self.description,
            server_id: &self.server_id,
            credential_owner: "caller",
            credential_stored: false,
        }
    }
}

// LiteLLM's five-key detail; credential_owner and credential_stored tell a client that nothing was saved.
#[derive(Debug, Serialize)]
pub struct BindingDenialDetail<'denial> {
    pub error: RejectionCode,
    pub error_description: &'denial str,
    pub server_id: &'denial str,
    pub credential_owner: &'static str,
    pub credential_stored: bool,
}
