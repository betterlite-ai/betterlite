use std::fmt;
use std::str::FromStr as _;

use rustls::pki_types::pem::Error as PemError;
use tokio_postgres::Config;
use tokio_postgres::config::SslMode;
use tokio_postgres_rustls_improved::MakeRustlsConnect;

use crate::database_url_policy::url_codec::{
    UrlSplitError, parse_qsl, urlencode_pairs_quote, urlsplit, urlunsplit,
};

pub(super) mod tls_config;

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
    #[error("pgbouncer must be true or false, got {value:?}")]
    PgbouncerFlag { value: String },
    #[error("the database URL is invalid: {source}")]
    MalformedUrl {
        #[source]
        source: UrlSplitError,
    },
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
}

// Maps a Prisma- or libpq-dialect URL to the tokio-postgres Config plus the rustls connector.
pub fn native_connection_options(
    database_url: &str,
) -> Result<NativeConnectionOptions, ConnectionOptionsError> {
    let mut parts =
        urlsplit(database_url).map_err(|source| ConnectionOptionsError::MalformedUrl { source })?;
    let pairs = parse_qsl(&parts.query, true);
    let request = tls_request(&pairs)?;
    let tls = MakeRustlsConnect::new(request.client_config()?);
    parts.query = urlencode_pairs_quote(&request.connection_pairs);
    let mut config = Config::from_str(&urlunsplit(&parts))
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
    // Prisma's query parser lets the last duplicate key win, so the native mapping does too
    let value = |key: &str| {
        pairs
            .iter()
            .rev()
            .find(|(existing, _)| existing == key)
            .map(|(_, found)| found.clone())
    };
    let sslmode = value("sslmode");
    let sslaccept = value("sslaccept");
    let sslcert = value("sslcert");
    let sslkey = value("sslkey");
    reject_unsupported_identity(pairs)?;
    let has_root = pairs.iter().any(|(key, _)| key == "sslrootcert");
    let mode = tls_mode(sslmode.as_deref());
    let verifies_mode = matches!(sslmode.as_deref(), Some("verify-ca" | "verify-full"));
    let verify = mode != SslMode::Disable
        && sslaccept
            .as_deref()
            .map_or(verifies_mode || has_root, |accept| {
                accept != "accept_invalid_certs"
            });
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

fn reject_unsupported_identity(pairs: &[(String, String)]) -> Result<(), ConnectionOptionsError> {
    if pairs.iter().any(|(key, _)| key == "sslidentity") {
        return Err(ConnectionOptionsError::UnsupportedIdentity);
    }
    if pairs.iter().any(|(key, _)| key == "sslpassword") {
        return Err(ConnectionOptionsError::EncryptedKey);
    }
    Ok(())
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

// Prisma reads an unknown or blank sslmode as prefer, so it never turns into a startup error here.
fn tls_mode(value: Option<&str>) -> SslMode {
    match value {
        Some("disable") => SslMode::Disable,
        Some("require" | "verify-ca" | "verify-full") => SslMode::Require,
        _ => SslMode::Prefer,
    }
}

// Prisma's pgbouncer=true means a transaction-pooling pooler, which cannot keep server-side prepared statements.
fn pgbouncer_flag(pairs: &[(String, String)]) -> Result<bool, ConnectionOptionsError> {
    let Some(flag) = pairs
        .iter()
        .rev()
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
