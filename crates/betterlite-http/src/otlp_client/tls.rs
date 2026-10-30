use std::path::{Path, PathBuf};
use std::sync::Arc;

use betterlite_config::sections::billing_metrics::Metering;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::pem::{self, PemObject as _};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TlsError {
    #[error("could not read PEM from {}: {source}", path.display())]
    Pem {
        path: PathBuf,
        #[source]
        source: pem::Error,
    },
    #[error("{} holds no certificate", path.display())]
    NoCertificate { path: PathBuf },
    #[error("no trusted CA certificate is available to verify the collector")]
    NoTrustRoots,
    #[error("rejected the CA certificate in {}: {source}", path.display())]
    Authority {
        path: PathBuf,
        #[source]
        source: rustls::Error,
    },
    #[error("could not set up mutual TLS: {source}")]
    Tls {
        #[source]
        source: rustls::Error,
    },
}

pub(super) fn tls_config(metering: &Metering) -> Result<ClientConfig, TlsError> {
    let certificates = certificates(&metering.client_cert)?;
    if certificates.is_empty() {
        return Err(TlsError::NoCertificate {
            path: metering.client_cert.clone(),
        });
    }
    let key =
        PrivateKeyDer::from_pem_file(&metering.client_key).map_err(|source| TlsError::Pem {
            path: metering.client_key.clone(),
            source,
        })?;
    let roots = trust_roots(metering.ca_cert.as_deref())?;
    ClientConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|source| TlsError::Tls { source })?
        .with_root_certificates(roots)
        .with_client_auth_cert(certificates, key)
        .map_err(|source| TlsError::Tls { source })
}

fn certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let unreadable = |source| TlsError::Pem {
        path: path.to_path_buf(),
        source,
    };
    CertificateDer::pem_file_iter(path)
        .map_err(unreadable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(unreadable)
}

fn trust_roots(authority: Option<&Path>) -> Result<RootCertStore, TlsError> {
    let mut roots = RootCertStore::empty();
    if let Some(path) = authority {
        for certificate in certificates(path)? {
            roots
                .add(certificate)
                .map_err(|source| TlsError::Authority {
                    path: path.to_path_buf(),
                    source,
                })?;
        }
    } else {
        let native = rustls_native_certs::load_native_certs();
        for error in native.errors {
            tracing::warn!("Billing metrics: skipped part of the system trust store: {error}");
        }
        roots.add_parsable_certificates(native.certs);
    }
    if roots.is_empty() {
        return Err(TlsError::NoTrustRoots);
    }
    Ok(roots)
}
