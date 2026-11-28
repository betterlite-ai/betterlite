use std::collections::BTreeMap;

use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use thiserror::Error;

use super::decode::DecodeError;
use crate::jwt_keys::errors::{KeyError, exception_for};

// The failures JWTHandler.auth_jwt raises: a key fetch error keeps B761's 503/401, everything else is a 401.
#[derive(Debug, Error)]
pub enum VerificationError {
    #[error("Token Expired")]
    Expired,
    #[error("Validation fails: {detail}")]
    Validation { detail: String },
    #[error("Invalid JWT Submitted")]
    InvalidJwt,
    #[error(transparent)]
    Key(#[from] KeyError),
    // get_unverified_header runs before the try block, so its PyJWT error is a bare authentication failure
    #[error("{detail}")]
    Header { detail: String },
}

impl From<DecodeError> for VerificationError {
    fn from(error: DecodeError) -> Self {
        Self::Header {
            detail: pyjwt_message(&error).to_owned(),
        }
    }
}

// PyJWT's own wording for a token whose segments or header never parse, which the mapper prefixes unchanged.
const fn pyjwt_message(error: &DecodeError) -> &'static str {
    match error {
        DecodeError::TooLarge { .. } => "the signed token is over the 1048576-byte limit",
        DecodeError::TooDeep { .. } => "the signed token nests deeper than 64 JSON levels",
        DecodeError::Utf8(_) | DecodeError::NotDocument | DecodeError::Document(_) => {
            "Invalid token"
        }
        DecodeError::Segments => "Not enough segments",
        DecodeError::Header => "Invalid header string: the header is not valid JSON",
        DecodeError::Claims(_) => "Invalid payload string: the payload is not valid base64url JSON",
    }
}

// _as_proxy_exception turns a generic Exception into 401 auth_error with param "None"; an expired token keeps its own type.
#[must_use]
pub fn into_proxy_exception(error: &VerificationError) -> ProxyException {
    match error {
        VerificationError::Key(key_error) => exception_for(key_error),
        VerificationError::Expired => ProxyException::new(ProxyExceptionInit {
            message: "Token Expired".to_owned(),
            error_type: "expired_key".to_owned(),
            param: None,
            code: ExceptionCode::Status(401),
            headers: BTreeMap::new(),
            openai_code: None,
            provider_specific_fields: BTreeMap::new(),
        }),
        VerificationError::Validation { detail } => {
            authentication_error(format!("Authentication Error, Validation fails: {detail}"))
        }
        VerificationError::InvalidJwt => {
            authentication_error("Authentication Error, Invalid JWT Submitted".to_owned())
        }
        VerificationError::Header { detail } => {
            authentication_error(format!("Authentication Error, {detail}"))
        }
    }
}

fn authentication_error(message: String) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message,
        error_type: "auth_error".to_owned(),
        param: Some("None".to_owned()),
        code: ExceptionCode::Status(401),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}
