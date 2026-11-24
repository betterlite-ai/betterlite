use std::collections::BTreeMap;

use crate::credential_contracts::errors::CredError;
use crate::wire_compat::errors::{HttpException, url_credentials_exception};

#[must_use]
pub fn raise_public(error: &CredError) -> HttpException {
    match error {
        CredError::Unauthorized(challenge) => {
            let detail = error.to_string();
            let headers = challenge
                .www_authenticate
                .as_ref()
                .map(|value| BTreeMap::from([("www-authenticate".to_owned(), value.clone())]))
                .unwrap_or_default();
            HttpException {
                status_code: 401,
                detail,
                headers,
            }
        }
        CredError::Misconfigured(_reason) => plain(500, error),
        CredError::UrlCredentialsNotAllowed => url_credentials_exception(),
        CredError::UpstreamUnavailable(_reason) => plain(503, error),
        CredError::UnsupportedMode(_reason) => plain(500, error),
        CredError::PreconditionRequired(_reason) => plain(412, error),
        CredError::NotImplemented(_reason) => plain(501, error),
    }
}

fn plain(status_code: u16, error: &CredError) -> HttpException {
    let detail = error.to_string();
    HttpException {
        status_code,
        detail,
        headers: BTreeMap::new(),
    }
}
