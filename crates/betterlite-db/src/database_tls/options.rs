use std::fmt;
use std::str::FromStr as _;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{VerifierBuilderError, WantsClientCert, WebPkiServerVerifier};
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::pem::Error as PemError;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime, pem::PemObject as _};
use rustls::{ClientConfig, ConfigBuilder, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio_postgres::Config;
use tokio_postgres::config::SslMode;
use tokio_postgres_rustls_improved::MakeRustlsConnect;

use super::url_query::{encode_pairs, query_pairs, split_url, unsplit};

const TLS_QUERY_KEYS: [&str; 7] = [
    "sslaccept",
    "sslcert",
    "sslidentity",
    "sslkey",
    "sslpassword",
    "sslrootcert",
    "sslmode",
];
// Prisma's own pool params are applied by the pool layer, not by a single connection URL.
const POOL_QUERY_KEYS: [&str; 6] = [
    "connection_limit",
    "max_idle_connection_lifetime",
    "pool_timeout",
    "pgbouncer",
    "schema",
    "socket_timeout",
];

pub struct NativeConnectionOptions {
    pub config: Config,
    pub tls: MakeRustlsConnect,
    pub disable_prepared_statements: bool,
}

impl fmt::Debug for NativeConnectionOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // the connector holds the trust anchors, never a caller secret; Config already redacts the password
        f.debug_struct("NativeConnectionOptions")
            .field("config", &self.config)
            .field(
                "disable_prepared_statements",
                &self.disable_prepared_statements,
            )
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionOptionsError {
    #[error("sslmode {value:?} is not a supported PostgreSQL TLS mode")]
    SslMode { value: String },
    #[error("pgbouncer must be true or false, got {value:?}")]
    PgbouncerFlag { value: String },
    #[error("sslfile {path} is not valid PEM: {source}")]
    Pem {
        path: String,
        #[source]
        source: PemError,
    },
    #[error("certificate in {path} was rejected: {source}")]
    Certificate {
        path: String,
        #[source]
        source: rustls::Error,
    },
    #[error("{path} contains no CA certificate")]
    EmptyRoots { path: String },
    #[error("{path} contains no client certificate")]
    EmptyCertificate { path: String },
    #[error("the system trust store could not be loaded: {source}")]
    NativeRoots {
        #[source]
        source: rustls_native_certs::Error,
    },
    #[error("the system trust store has no CA certificates")]
    NoNativeRoots,
    #[error("sslkey {path} was supplied without sslcert")]
    MissingClientCertificate { path: String },
    #[error("Prisma's PKCS#12 client identity (sslidentity) is not supported")]
    UnsupportedIdentity,
    #[error("sslpassword is not supported; client keys must be unencrypted PEM")]
    EncryptedKey,
    #[error("the connection URL could not be parsed: {source}")]
    Url {
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("the TLS protocol versions could not be configured: {source}")]
    ProtocolVersions {
        #[source]
        source: rustls::Error,
    },
    #[error("the client TLS configuration was rejected: {source}")]
    ClientConfig {
        #[source]
        source: rustls::Error,
    },
    #[error("the signature verifiers could not be configured: {source}")]
    SignatureVerifiers {
        #[source]
        source: VerifierBuilderError,
    },
}

// Maps a Prisma- or libpq-dialect URL to the tokio-postgres Config plus the rustls connector.
pub fn native_connection_options(
    database_url: &str,
) -> Result<NativeConnectionOptions, ConnectionOptionsError> {
    let parts = split_url(database_url);
    let pairs = query_pairs(&parts.query, true);
    let request = tls_request(&pairs)?;
    let tls = MakeRustlsConnect::new(request.client_config()?);
    let connection_query = encode_pairs(&request.connection_pairs, false);
    let mut config = Config::from_str(&unsplit(&parts, &connection_query))
        .map_err(|source| ConnectionOptionsError::Url { source })?;
    config.ssl_mode(request.mode);
    Ok(NativeConnectionOptions {
        config,
        tls,
        disable_prepared_statements: request.disable_prepared_statements,
    })
}

struct TlsRequest {
    mode: SslMode,
    verify: bool,
    root_paths: Vec<String>,
    ca_path: Option<String>,
    identity: Option<(String, String)>,
    connection_pairs: Vec<(String, String)>,
    disable_prepared_statements: bool,
}

fn tls_request(pairs: &[(String, String)]) -> Result<TlsRequest, ConnectionOptionsError> {
    let value = |key: &str| {
        pairs
            .iter()
            .find(|(existing, _)| existing == key)
            .map(|(_, found)| found.clone())
    };
    let sslmode = value("sslmode");
    let sslaccept = value("sslaccept");
    let sslcert = value("sslcert");
    let sslkey = value("sslkey");
    if pairs.iter().any(|(key, _)| key == "sslidentity") {
        return Err(ConnectionOptionsError::UnsupportedIdentity);
    }
    if pairs.iter().any(|(key, _)| key == "sslpassword") {
        return Err(ConnectionOptionsError::EncryptedKey);
    }
    let mode = tls_mode(sslmode.as_deref())?;
    let verifies_mode = matches!(sslmode.as_deref(), Some("verify-ca" | "verify-full"));
    let verify = mode != SslMode::Disable
        && sslaccept
            .as_deref()
            .map_or(verifies_mode, |accept| accept == "strict");
    let (identity, ca_path) = identity_paths(sslcert, sslkey)?;
    let root_paths = pairs
        .iter()
        .filter(|(key, _)| key == "sslrootcert")
        .map(|(_, root)| root.clone())
        .collect();
    Ok(TlsRequest {
        mode,
        verify,
        root_paths,
        ca_path,
        identity,
        connection_pairs: connection_pairs(pairs),
        disable_prepared_statements: pgbouncer_flag(pairs)?,
    })
}

type ClientIdentityPaths = (Option<(String, String)>, Option<String>);

fn identity_paths(
    sslcert: Option<String>,
    sslkey: Option<String>,
) -> Result<ClientIdentityPaths, ConnectionOptionsError> {
    match (sslcert, sslkey) {
        (Some(cert), Some(key)) => Ok((Some((cert, key)), None)),
        (None, Some(key)) => Err(ConnectionOptionsError::MissingClientCertificate { path: key }),
        (Some(cert), None) => Ok((None, Some(cert))),
        (None, None) => Ok((None, None)),
    }
}

// Everything tokio-postgres reads stays; the TLS params and Prisma's pool params are handled here or by the pool layer.
fn connection_pairs(pairs: &[(String, String)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .filter(|(key, _)| {
            !TLS_QUERY_KEYS.contains(&key.as_str()) && !POOL_QUERY_KEYS.contains(&key.as_str())
        })
        .cloned()
        .collect()
}

fn tls_mode(value: Option<&str>) -> Result<SslMode, ConnectionOptionsError> {
    match value {
        None | Some("prefer") => Ok(SslMode::Prefer),
        Some("disable") => Ok(SslMode::Disable),
        Some("require" | "verify-ca" | "verify-full") => Ok(SslMode::Require),
        Some(other) => Err(ConnectionOptionsError::SslMode {
            value: other.to_owned(),
        }),
    }
}

// Prisma's pgbouncer=true means a transaction-pooling pooler, which cannot keep server-side prepared statements.
fn pgbouncer_flag(pairs: &[(String, String)]) -> Result<bool, ConnectionOptionsError> {
    let Some(flag) = pairs
        .iter()
        .find(|(key, _)| key == "pgbouncer")
        .map(|(_, value)| value.as_str())
    else {
        return Ok(false);
    };
    match flag {
        "false" => Ok(false),
        "true" => Ok(true),
        other => Err(ConnectionOptionsError::PgbouncerFlag {
            value: other.to_owned(),
        }),
    }
}

impl TlsRequest {
    fn client_config(&self) -> Result<ClientConfig, ConnectionOptionsError> {
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
