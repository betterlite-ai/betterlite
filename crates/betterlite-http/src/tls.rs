use std::fs;
use std::io::Error as IoError;
use std::num::TryFromIntError;
use std::sync::Arc;

use crate::error::HttpError;
use betterlite_config::tls::{TlsPolicy, Verification};
use reqwest::header::{HeaderValue, InvalidHeaderValue};
use reqwest::{Certificate, ClientBuilder};
use rustls::pki_types::{CertificateDer, pem::PemObject as _};
use rustls::{ClientConfig, NamedGroup, RootCertStore};

const MAX_CA_PEM_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error(transparent)]
    Policy(#[from] betterlite_config::tls::TlsPolicyError),
    #[error("cannot read SSL_CERT_FILE")]
    ReadCa(#[source] IoError),
    #[error("SSL_CERT_FILE length is unsupported")]
    CaLength(#[source] TryFromIntError),
    #[error("SSL_CERT_FILE is larger than the supported limit")]
    CaTooLarge,
    #[error("SSL_CERT_FILE contains invalid PEM")]
    InvalidCa(#[source] reqwest::Error),
    #[error("SSL_CERT_FILE contains no certificates")]
    EmptyCa,
    #[error("SSL_ECDH_CURVE {0} is not supported by the selected crypto provider")]
    UnsupportedCurve(String),
    #[error("SSL_CERT_FILE contains invalid PEM")]
    InvalidPem(#[source] rustls::pki_types::pem::Error),
    #[error("could not build the rustls client configuration")]
    RustlsConfig(#[source] rustls::Error),
    #[error("LITELLM_USER_AGENT is not a valid HTTP header value")]
    UserAgent(#[source] InvalidHeaderValue),
}

pub fn apply_policy(
    mut builder: ClientBuilder,
    policy: &TlsPolicy,
) -> Result<ClientBuilder, TlsError> {
    let mut ca_pem = None;
    if matches!(policy.verification, Verification::Disabled) {
        builder = builder.tls_danger_accept_invalid_certs(true);
    }
    if let Some(path) = &policy.ca_file {
        let metadata = fs::metadata(path).map_err(TlsError::ReadCa)?;
        if metadata.len() > MAX_CA_PEM_BYTES {
            return Err(TlsError::CaTooLarge);
        }
        let pem = fs::read(path).map_err(TlsError::ReadCa)?;
        if u64::try_from(pem.len()).map_err(TlsError::CaLength)? > MAX_CA_PEM_BYTES {
            return Err(TlsError::CaTooLarge);
        }
        let certificates = Certificate::from_pem_bundle(&pem).map_err(TlsError::InvalidCa)?;
        if certificates.is_empty() {
            return Err(TlsError::EmptyCa);
        }
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
        ca_pem = Some(pem);
    }
    let user_agent = HeaderValue::from_str(&policy.user_agent).map_err(TlsError::UserAgent)?;
    builder = builder
        .user_agent(user_agent)
        .http2_adaptive_window(policy.http2);
    if let Some(curve) = &policy.ecdh_curve {
        if let Some(group) = named_group(curve) {
            builder = builder.use_preconfigured_tls(client_config(group, ca_pem.as_deref())?);
        } else {
            tracing::warn!(
                curve,
                "unrecognized SSL_ECDH_CURVE; using provider defaults"
            );
        }
    }
    Ok(builder)
}

fn named_group(name: &str) -> Option<NamedGroup> {
    match name {
        "X25519" => Some(NamedGroup::X25519),
        "prime256v1" => Some(NamedGroup::secp256r1),
        "secp384r1" => Some(NamedGroup::secp384r1),
        _ => None,
    }
}

fn client_config(group: NamedGroup, ca_pem: Option<&[u8]>) -> Result<ClientConfig, TlsError> {
    let mut provider = rustls::crypto::aws_lc_rs::default_provider();
    let selected = provider
        .kx_groups
        .iter()
        .find(|candidate| candidate.name() == group)
        .copied()
        .ok_or_else(|| TlsError::UnsupportedCurve(format!("{group:?}")))?;
    provider.kx_groups = vec![selected];
    let mut roots = RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    for error in native.errors {
        tracing::warn!(%error, "could not load part of the system trust store");
    }
    roots.add_parsable_certificates(native.certs);
    if let Some(pem) = ca_pem {
        let certificates = CertificateDer::pem_slice_iter(pem)
            .collect::<Result<Vec<_>, _>>()
            .map_err(TlsError::InvalidPem)?;
        for certificate in certificates {
            roots.add(certificate).map_err(TlsError::RustlsConfig)?;
        }
    }
    ClientConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()
        .map_err(TlsError::RustlsConfig)
        .map(|builder| builder.with_root_certificates(roots).with_no_client_auth())
}

mod tests;
