use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{WantsClientCert, WebPkiServerVerifier};
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::pem::Error as PemError;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime, pem::PemObject as _};
use rustls::{ClientConfig, ConfigBuilder, DigitallySignedStruct, RootCertStore, SignatureScheme};

use super::{ConnectionOptionsError, TlsRequest};

impl TlsRequest {
    pub(super) fn client_config(&self) -> Result<ClientConfig, ConnectionOptionsError> {
        let provider = Arc::new(aws_lc_rs::default_provider());
        let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .map_err(|source| ConnectionOptionsError::ProtocolVersions { source })?;
        let identity = match &self.identity {
            Some((cert, key)) => Some(client_identity(cert, key)?),
            None => None,
        };
        if self.verify {
            let roots = self.roots()?;
            return finish_config(builder.with_root_certificates(roots), identity);
        }
        let signatures =
            WebPkiServerVerifier::builder_with_provider(Arc::new(RootCertStore::empty()), provider)
                .build()
                .map_err(|source| ConnectionOptionsError::SignatureVerifiers { source })?;
        let verifier = Arc::new(AcceptAnyServerCert { signatures });
        let dangerous = builder
            .dangerous()
            .with_custom_certificate_verifier(verifier);
        finish_config(dangerous, identity)
    }

    fn roots(&self) -> Result<RootCertStore, ConnectionOptionsError> {
        let mut store = RootCertStore::empty();
        if self.root_paths.is_empty() {
            match &self.ca_path {
                Some(path) => add_roots(&mut store, path)?,
                None => add_native_roots(&mut store)?,
            }
        } else {
            for path in &self.root_paths {
                add_roots(&mut store, path)?;
            }
        }
        Ok(store)
    }
}

fn finish_config(
    builder: ConfigBuilder<ClientConfig, WantsClientCert>,
    identity: Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>,
) -> Result<ClientConfig, ConnectionOptionsError> {
    match identity {
        Some((chain, key)) => builder
            .with_client_auth_cert(chain, key)
            .map_err(|source| ConnectionOptionsError::ClientConfig { source }),
        None => Ok(builder.with_no_client_auth()),
    }
}

fn parse_pem_bundle(path: &str) -> Result<Vec<CertificateDer<'static>>, ConnectionOptionsError> {
    let certificates =
        CertificateDer::pem_file_iter(path).map_err(|source| ConnectionOptionsError::Pem {
            path: path.to_owned(),
            source,
        })?;
    certificates
        .collect::<Result<_, PemError>>()
        .map_err(|source| ConnectionOptionsError::Pem {
            path: path.to_owned(),
            source,
        })
}

// Every certificate in the configured bundle is trusted, so a multi-root bundle needs no probe or temp file.
fn add_roots(store: &mut RootCertStore, path: &str) -> Result<(), ConnectionOptionsError> {
    let certificates = parse_pem_bundle(path)?;
    if certificates.is_empty() {
        return Err(ConnectionOptionsError::EmptyRoots {
            path: path.to_owned(),
        });
    }
    insert_certificates(store, certificates, path)
}

fn add_native_roots(store: &mut RootCertStore) -> Result<(), ConnectionOptionsError> {
    let native = rustls_native_certs::load_native_certs();
    if let Some(source) = native.errors.into_iter().next() {
        return Err(ConnectionOptionsError::NativeRoots { source });
    }
    if native.certs.is_empty() {
        return Err(ConnectionOptionsError::NoNativeRoots);
    }
    insert_certificates(store, native.certs, "system trust store")
}

fn insert_certificates(
    store: &mut RootCertStore,
    certificates: Vec<CertificateDer<'static>>,
    path: &str,
) -> Result<(), ConnectionOptionsError> {
    for certificate in certificates {
        store
            .add(certificate)
            .map_err(|source| ConnectionOptionsError::Certificate {
                path: path.to_owned(),
                source,
            })?;
    }
    Ok(())
}

fn client_identity(
    certificate_path: &str,
    key_path: &str,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>), ConnectionOptionsError> {
    let chain = parse_pem_bundle(certificate_path)?;
    if chain.is_empty() {
        return Err(ConnectionOptionsError::EmptyCertificate {
            path: certificate_path.to_owned(),
        });
    }
    let key =
        PrivateKeyDer::from_pem_file(key_path).map_err(|source| ConnectionOptionsError::Pem {
            path: key_path.to_owned(),
            source,
        })?;
    Ok((chain, key))
}

// sslmode=require and Prisma's default accept_invalid_certs encrypt without authenticating the server.
#[derive(Debug)]
struct AcceptAnyServerCert {
    signatures: Arc<WebPkiServerVerifier>,
}

macro_rules! verify_signature {
    ($method:ident) => {
        fn $method(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            self.signatures.$method(message, cert, dss)
        }
    };
}

impl ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    verify_signature!(verify_tls12_signature);
    verify_signature!(verify_tls13_signature);

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.signatures.supported_verify_schemes()
    }
}
