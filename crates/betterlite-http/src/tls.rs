use std::fs::File;
use std::io::{Error as IoError, Read as _};
use std::num::TryFromIntError;
use std::sync::Arc;

use betterlite_config::tls::{TlsPolicy, TlsPolicyError, Verification};
use reqwest::ClientBuilder;
use reqwest::header::{HeaderValue, InvalidHeaderValue};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{VerifierBuilderError, WebPkiServerVerifier};
use rustls::crypto::{CryptoProvider, SupportedKxGroup, aws_lc_rs};
use rustls::pki_types::{
    CertificateDer, ServerName, UnixTime, pem::Error as PemError, pem::PemObject as _,
};
use rustls::{
    ClientConfig, DigitallySignedStruct, Error as RustlsError, NamedGroup, RootCertStore,
    SignatureScheme,
};

use crate::error::HttpError;

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
    #[error("SSL_CERT_FILE contains no certificates")]
    EmptyCa,
    #[error("SSL_ECDH_CURVE {0} is not supported by the selected crypto provider")]
    UnsupportedCurve(String),
    #[error("SSL_CERT_FILE contains invalid PEM")]
    InvalidPem(#[source] PemError),
    #[error("SSL_CERT_FILE contains an invalid certificate")]
    InvalidDer(#[source] rustls::Error),
    #[error("could not build the rustls client configuration")]
    RustlsConfig(#[source] rustls::Error),
    #[error("could not configure TLS handshake signature checks")]
    Signatures(#[source] VerifierBuilderError),
    #[error("LITELLM_USER_AGENT is not a valid HTTP header value")]
    UserAgent(#[source] InvalidHeaderValue),
}

pub fn apply_policy(
    initial_builder: ClientBuilder,
    policy: &TlsPolicy,
) -> Result<ClientBuilder, TlsError> {
    let mut config = client_config(policy)?;
    config.alpn_protocols = if policy.http2 {
        vec![b"h2".to_vec(), b"http/1.1".to_vec()]
    } else {
        vec![b"http/1.1".to_vec()]
    };
    let user_agent = HeaderValue::from_str(&policy.user_agent).map_err(TlsError::UserAgent)?;
    let builder = initial_builder
        .tls_backend_preconfigured(config)
        .user_agent(user_agent);
    Ok(builder)
}

fn trust_roots(ca_file: Option<&str>) -> Result<RootCertStore, TlsError> {
    let Some(path) = ca_file else {
        return Ok(RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        });
    };
    let file = File::open(path).map_err(TlsError::ReadCa)?;
    let metadata = file.metadata().map_err(TlsError::ReadCa)?;
    if metadata.len() > MAX_CA_PEM_BYTES {
        return Err(TlsError::CaTooLarge);
    }
    let mut pem = Vec::new();
    file.take(MAX_CA_PEM_BYTES.saturating_add(1))
        .read_to_end(&mut pem)
        .map_err(TlsError::ReadCa)?;
    parse_ca_pem(&pem)
}

pub fn parse_ca_pem(pem: &[u8]) -> Result<RootCertStore, TlsError> {
    if u64::try_from(pem.len()).map_err(TlsError::CaLength)? > MAX_CA_PEM_BYTES {
        return Err(TlsError::CaTooLarge);
    }
    let mut roots = RootCertStore::empty();
    for certificate in CertificateDer::pem_slice_iter(pem) {
        roots
            .add(certificate.map_err(TlsError::InvalidPem)?)
            .map_err(TlsError::InvalidDer)?;
    }
    if roots.is_empty() {
        return Err(TlsError::EmptyCa);
    }
    Ok(roots)
}

fn named_group(name: &str) -> Option<NamedGroup> {
    match name {
        "X25519" => Some(NamedGroup::X25519),
        "prime256v1" => Some(NamedGroup::secp256r1),
        "secp384r1" => Some(NamedGroup::secp384r1),
        "secp521r1" => Some(NamedGroup::secp521r1),
        _ => None,
    }
}

fn client_config(policy: &TlsPolicy) -> Result<ClientConfig, TlsError> {
    let mut provider = aws_lc_rs::default_provider();
    if let Some(curve) = &policy.ecdh_curve {
        if let Some(group) = named_group(curve) {
            provider.kx_groups = select_kx_groups(group, curve, &provider)?;
        } else {
            tracing::warn!(
                curve,
                "unrecognized SSL_ECDH_CURVE; using provider defaults"
            );
        }
    }
    let shared_provider = Arc::new(provider);
    let builder = ClientConfig::builder_with_provider(Arc::clone(&shared_provider))
        .with_safe_default_protocol_versions()
        .map_err(TlsError::RustlsConfig)?;
    if matches!(policy.verification, Verification::Disabled) {
        let signatures = WebPkiServerVerifier::builder_with_provider(
            Arc::new(trust_roots(None)?),
            shared_provider,
        )
        .build()
        .map_err(TlsError::Signatures)?;
        let verifier = InsecureCertificateVerifier { signatures };
        return Ok(builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth());
    }
    let roots = trust_roots(policy.ca_file.as_deref())?;
    Ok(builder.with_root_certificates(roots).with_no_client_auth())
}

#[derive(Debug)]
struct InsecureCertificateVerifier {
    signatures: Arc<WebPkiServerVerifier>,
}

macro_rules! verify_signature {
    ($method:ident) => {
        fn $method(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, RustlsError> {
            self.signatures.$method(message, cert, dss)
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

    verify_signature!(verify_tls12_signature);
    verify_signature!(verify_tls13_signature);

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.signatures.supported_verify_schemes()
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
