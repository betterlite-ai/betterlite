mod tests;

use std::str::FromStr as _;

use super::errors::{CredError, CredResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum AuthSpecKind {
    AuthorizationCode,
    ClientCredentials,
    TokenExchange,
    IdJag,
    ApiKey,
    Passthrough,
    None,
    AwsSigv4,
}

pub fn parse_auth_spec_kind(raw: &str) -> CredResult<AuthSpecKind> {
    match AuthSpecKind::from_str(raw) {
        Ok(kind) => Ok(kind),
        Err(_unused) => Err(CredError::UnsupportedMode(format!(
            "unknown auth_spec_kind: '{raw}'"
        ))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum AuthResolution {
    #[strum(serialize = "no-auth")]
    NoAuth,
    #[strum(serialize = "stored-user-token")]
    StoredUserToken,
    #[strum(serialize = "static-token")]
    StaticToken,
    #[strum(serialize = "per-request-header")]
    PerRequestHeader,
    #[strum(serialize = "oauth2-passthrough")]
    Oauth2Passthrough,
    #[strum(serialize = "m2m-client-credentials")]
    ClientCredentials,
    #[strum(serialize = "token-exchange")]
    TokenExchange,
    #[strum(serialize = "id-jag")]
    IdJag,
    #[strum(serialize = "aws-sigv4")]
    AwsSigv4,
    #[strum(serialize = "extra-headers")]
    ExtraHeaders,
    #[strum(serialize = "not-applicable")]
    NotApplicable,
    #[strum(serialize = "unresolved")]
    Unresolved,
    #[strum(serialize = "resolution-failed")]
    Failed,
    #[strum(serialize = "multiple")]
    Multiple,
}
