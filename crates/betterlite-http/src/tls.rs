use std::fs;

use betterlite_config::tls::{TlsPolicy, Verification};
use reqwest::{Certificate, ClientBuilder};

const MAX_CA_PEM_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("cannot read SSL_CERT_FILE")]
    ReadCa(#[source] std::io::Error),
    #[error("SSL_CERT_FILE is larger than the supported limit")]
    CaTooLarge,
    #[error("SSL_CERT_FILE contains invalid PEM")]
    InvalidCa(#[source] reqwest::Error),
    #[error("SSL_CERT_FILE contains no certificates")]
    EmptyCa,
    #[error("LITELLM_USER_AGENT is not a valid HTTP header value")]
    UserAgent(#[source] reqwest::header::InvalidHeaderValue),
}

pub fn apply_policy(
    mut builder: ClientBuilder,
    policy: &TlsPolicy,
) -> Result<ClientBuilder, TlsError> {
    if matches!(policy.verification, Verification::Disabled) {
        builder = builder.tls_danger_accept_invalid_certs(true);
    }
    if let Some(path) = &policy.ca_file {
        let metadata = fs::metadata(path).map_err(TlsError::ReadCa)?;
        if metadata.len() > MAX_CA_PEM_BYTES {
            return Err(TlsError::CaTooLarge);
        }
        let pem = fs::read(path).map_err(TlsError::ReadCa)?;
        if u64::try_from(pem.len()).map_err(|_| TlsError::CaTooLarge)? > MAX_CA_PEM_BYTES {
            return Err(TlsError::CaTooLarge);
        }
        let certificates = Certificate::from_pem_bundle(&pem).map_err(TlsError::InvalidCa)?;
        if certificates.is_empty() {
            return Err(TlsError::EmptyCa);
        }
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
    }
    let user_agent =
        reqwest::header::HeaderValue::from_str(&policy.user_agent).map_err(TlsError::UserAgent)?;
    builder = builder
        .user_agent(user_agent)
        .http2_adaptive_window(policy.http2);
    Ok(builder)
}

mod tests;
