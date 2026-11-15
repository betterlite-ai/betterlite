use betterlite_mcp::credential_contracts::errors::CredError;

use crate::helpers::HelperError;

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            Box::new(HelperError {
                class: "RuntimeError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
}

pub(super) const fn cred_tag(error: &CredError) -> &'static str {
    match error {
        CredError::Unauthorized(_) => "unauthorized",
        CredError::Misconfigured(_) => "misconfigured",
        CredError::UrlCredentialsNotAllowed => "url_credentials_not_allowed",
        CredError::UpstreamUnavailable(_) => "upstream_unavailable",
        CredError::UnsupportedMode(_) => "unsupported_mode",
        CredError::PreconditionRequired(_) => "precondition_required",
        CredError::NotImplemented(_) => "not_implemented",
    }
}

pub(super) fn is_hex_32(value: &str) -> bool {
    let allowed = value
        .chars()
        .all(|letter| letter.is_ascii_hexdigit() && !letter.is_ascii_uppercase());
    value.len() == 32 && allowed
}
