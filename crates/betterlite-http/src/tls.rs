use std::fs;
use std::io::Error as IoError;
use std::num::TryFromIntError;
use std::sync::Arc;

use crate::error::HttpError;
use betterlite_config::tls::{TlsPolicy, TlsPolicyError, Verification};
use reqwest::header::{HeaderValue, InvalidHeaderValue};
use reqwest::{Certificate, ClientBuilder};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::aws_lc_rs;
use rustls::crypto::{
    CryptoProvider, SupportedKxGroup, WebPkiSupportedAlgorithms, verify_tls12_signature,
    verify_tls13_signature,
};
use rustls::pki_types::{
    CertificateDer, ServerName, UnixTime, pem::Error as PemError, pem::PemObject as _,
};
use rustls::{
    ClientConfig, DigitallySignedStruct, Error as RustlsError, NamedGroup, RootCertStore,
    SignatureScheme,
};

const MAX_CA_PEM_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error(transparent)]
    Policy(#[from] TlsPolicyError),
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
    InvalidPem(#[source] PemError),
    #[error("could not build the rustls client configuration")]
    RustlsConfig(#[source] rustls::Error),
    #[error("LITELLM_USER_AGENT is not a valid HTTP header value")]
    UserAgent(#[source] InvalidHeaderValue),
}

pub fn apply_policy(
    initial_builder: ClientBuilder,
    policy: &TlsPolicy,
) -> Result<ClientBuilder, TlsError> {
    let (mut builder, ca_pem) = apply_ca_file(initial_builder, policy.ca_file.as_deref())?;
    if matches!(policy.verification, Verification::Disabled) {
        builder = builder.tls_danger_accept_invalid_certs(true);
    }
    let user_agent = HeaderValue::from_str(&policy.user_agent).map_err(TlsError::UserAgent)?;
    builder = builder
        .user_agent(user_agent)
        .http2_adaptive_window(policy.http2);
    if let Some(curve) = &policy.ecdh_curve {
        if let Some(group) = named_group(curve) {
            builder = builder.use_preconfigured_tls(client_config(
                group,
                curve,
                ca_pem.as_deref(),
                matches!(policy.verification, Verification::Disabled),
            )?);
        } else {
            tracing::warn!(
                curve,
                "unrecognized SSL_ECDH_CURVE; using provider defaults"
            );
        }
    }
    Ok(builder)
}

fn apply_ca_file(
    mut builder: ClientBuilder,
    ca_file: Option<&str>,
) -> Result<(ClientBuilder, Option<Vec<u8>>), TlsError> {
    let Some(path) = ca_file else {
        return Ok((builder, None));
    };
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
    Ok((builder, Some(pem)))
}

fn named_group(name: &str) -> Option<NamedGroup> {
    match name {
        "X25519" => Some(NamedGroup::X25519),
        "prime256v1" => Some(NamedGroup::secp256r1),
        "secp384r1" => Some(NamedGroup::secp384r1),
        _ => None,
    }
}

fn client_config(
    group: NamedGroup,
    curve_name: &str,
    ca_pem: Option<&[u8]>,
    verification_disabled: bool,
) -> Result<ClientConfig, TlsError> {
    let mut provider = aws_lc_rs::default_provider();
    let selected_groups = select_kx_groups(group, curve_name, &provider)?;
    provider.kx_groups = selected_groups;
    let CryptoProvider {
        signature_verification_algorithms: sig_algs,
        ..
    } = provider;
    let shared_algorithms = Arc::new(sig_algs);
    let builder = ClientConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()
        .map_err(TlsError::RustlsConfig)?;
    if verification_disabled {
        let verifier = InsecureCertificateVerifier {
            algorithms: shared_algorithms,
        };
        return Ok(builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth());
    }
    let roots = trust_roots(ca_pem)?;
    Ok(builder.with_root_certificates(roots).with_no_client_auth())
}

fn trust_roots(ca_pem: Option<&[u8]>) -> Result<RootCertStore, TlsError> {
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
    Ok(roots)
}

#[derive(Debug)]
struct InsecureCertificateVerifier {
    algorithms: Arc<WebPkiSupportedAlgorithms>,
}

macro_rules! verify_signature {
    ($method:ident, $verify:path) => {
        fn $method(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, RustlsError> {
            $verify(message, cert, dss, &self.algorithms)
        }
    };
}

impl ServerCertVerifier for InsecureCertificateVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }

    verify_signature!(verify_tls12_signature, verify_tls12_signature);
    verify_signature!(verify_tls13_signature, verify_tls13_signature);

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

fn select_kx_groups(
    group: NamedGroup,
    curve_name: &str,
    provider: &CryptoProvider,
) -> Result<Vec<&'static dyn SupportedKxGroup>, TlsError> {
    let selected = provider
        .kx_groups
        .iter()
        .find(|candidate| candidate.name() == group)
        .copied()
        .ok_or_else(|| TlsError::UnsupportedCurve(curve_name.to_owned()))?;
    Ok(vec![selected])
}

mod tests;
