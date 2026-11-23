use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_auth::user_roles::UserRole;
use betterlite_db::key_repository::TokenRepositoryError;
use betterlite_db::team_repository::decoder::TeamRecordError;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use crate::management_admins::ports::TeamReadError;
use crate::spend::tag_reports::store::TagReportError;
use crate::team_access::error::TeamAccessError;

// validate_key_list_check's caller-visible denials; B782 turns them into the HTTP response
#[derive(Debug, thiserror::Error)]
pub enum KeyListFiltersError {
    #[error(
        "You are not authorized to access this endpoint. No 'user_id' is associated with your API key."
    )]
    NoCallerRow,
    #[error("You are not authorized to check another user's keys")]
    ForeignUser,
    #[error("You are not authorized to check this team's keys")]
    ForeignTeam,
    #[error("You are not authorized to check this organization's keys")]
    ForeignOrganization,
    #[error("Key Hash not found.")]
    KeyHashNotFound,
    // LiteLLM raises a bare HTTPException(403) here, whose detail carries the caller's role
    #[error(
        "You are not allowed to access this key's info. Your role={}",
        role.map_or_else(|| "None".to_owned(), |role| role.to_string())
    )]
    KeyInfoForbidden { role: Option<UserRole> },
    #[error("a key-list row could not be read")]
    Read(#[from] KeyListReadError),
}

// the repository failures this module reads through, kept apart from the denials
#[derive(Debug, thiserror::Error)]
pub enum KeyListReadError {
    #[error("the caller's user row could not be read")]
    User(#[source] TagReportError),
    #[error("the key row could not be read")]
    Key(#[source] TokenRepositoryError<Infallible>),
    #[error("the user's team rows could not be read")]
    Teams(#[source] TeamReadError),
    #[error("a stored team row cannot be decoded")]
    Team(#[source] TeamRecordError),
    #[error("the caller's identity could not be read")]
    Caller(#[source] TeamAccessError),
    #[error("the key's access could not be checked")]
    Access(#[source] TeamAccessError),
}

impl KeyListFiltersError {
    // the HTTP status LiteLLM raises; None is a repository failure its handler maps
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::NoCallerRow
            | Self::ForeignUser
            | Self::ForeignTeam
            | Self::ForeignOrganization
            | Self::KeyHashNotFound
            | Self::KeyInfoForbidden { .. } => Some(403_u16),
            Self::Read(_) => None,
        }
    }

    // LiteLLM raises ProxyException(bad_request_error, param, 403) for every denial but the key-info one
    #[must_use]
    pub fn proxy_exception(&self) -> Option<ProxyException> {
        let param = match self {
            Self::NoCallerRow | Self::ForeignUser => "user_id",
            Self::ForeignTeam => "team_id",
            Self::ForeignOrganization => "organization_id",
            Self::KeyHashNotFound => "key_hash",
            Self::KeyInfoForbidden { .. } | Self::Read(_) => return None,
        };
        Some(ProxyException::new(ProxyExceptionInit {
            message: self.to_string(),
            error_type: ProxyErrorTypes::BadRequestError.to_string(),
            param: Some(param.to_owned()),
            code: ExceptionCode::Status(403_i64),
            headers: BTreeMap::new(),
            openai_code: None,
            provider_specific_fields: BTreeMap::new(),
        }))
    }
}
