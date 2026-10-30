use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use super::{RefreshCredential, UpstreamTokenGrant};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireGrant {
    pub(crate) access_token: String,
    pub(crate) token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expires_in: Option<i64>,
}

impl WireGrant {
    pub(crate) fn is_valid(&self) -> bool {
        if self.access_token.is_empty() || self.token_type.is_empty() {
            return false;
        }
        if self.refresh_token.as_ref().is_some_and(String::is_empty) {
            return false;
        }
        self.expires_in.is_none_or(|value| value > 0)
    }
}

impl From<UpstreamTokenGrant> for WireGrant {
    fn from(grant: UpstreamTokenGrant) -> Self {
        Self {
            access_token: grant.access_token.expose_secret().to_owned(),
            token_type: grant.token_type,
            refresh_token: grant
                .refresh_token
                .map(|value| value.expose_secret().to_owned()),
            scope: grant.scope,
            expires_in: grant.expires_in,
        }
    }
}

impl From<WireGrant> for UpstreamTokenGrant {
    fn from(wire: WireGrant) -> Self {
        Self {
            access_token: SecretString::from(wire.access_token),
            token_type: wire.token_type,
            refresh_token: wire.refresh_token.map(SecretString::from),
            scope: wire.scope,
            expires_in: wire.expires_in,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireRefresh {
    pub(crate) refresh_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expires_in: Option<i64>,
}

impl WireRefresh {
    pub(crate) fn is_valid(&self) -> bool {
        if self.refresh_token.is_empty() {
            return false;
        }
        self.expires_in.is_none_or(|value| value > 0)
    }
}

impl From<RefreshCredential> for WireRefresh {
    fn from(refresh: RefreshCredential) -> Self {
        Self {
            refresh_token: refresh.refresh_token.expose_secret().to_owned(),
            scope: refresh.scope,
            expires_in: refresh.expires_in,
        }
    }
}

impl From<WireRefresh> for RefreshCredential {
    fn from(wire: WireRefresh) -> Self {
        Self {
            refresh_token: SecretString::from(wire.refresh_token),
            scope: wire.scope,
            expires_in: wire.expires_in,
        }
    }
}
