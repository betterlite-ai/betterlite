use std::fs::File;
use std::io::BufReader;

use rustls::{ClientConfig, RootCertStore};
use tokio_postgres_rustls_improved::MakeRustlsConnect;

use super::ConnectionOptions;
use crate::error::FixtureError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TlsMode {
    Disable,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

pub(super) fn tls_mode(raw: &str) -> TlsMode {
    match query_value(raw, "sslmode").as_deref() {
        Some("disable") => TlsMode::Disable,
        Some("require") => TlsMode::Require,
        Some("verify-ca") => TlsMode::VerifyCa,
        Some("verify-full") => TlsMode::VerifyFull,
        _ => TlsMode::Prefer,
    }
}

pub(super) fn query_value(raw: &str, key: &str) -> Option<String> {
    raw.split_once('?')?.1.split('&').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        (name == key).then(|| value.to_owned())
    })
}

pub(super) fn normalized_config_source(raw: &str, tls_mode: TlsMode) -> String {
    let Some((base, query)) = raw.split_once('?') else {
        return raw.to_owned();
    };
    let parameters = query
        .split('&')
        .filter_map(|parameter| {
            let (name, configured_value) = parameter.split_once('=')?;
            if name == "sslrootcert" {
                return None;
            }
            let normalized_value = if name == "sslmode" && tls_mode != TlsMode::Disable {
                "require"
            } else {
                configured_value
            };
            Some(format!("{name}={normalized_value}"))
        })
        .collect::<Vec<_>>();
    if parameters.is_empty() {
        base.to_owned()
    } else {
        format!("{base}?{}", parameters.join("&"))
    }
}

pub(super) fn tls_connector(
    options: &ConnectionOptions,
) -> Result<MakeRustlsConnect, FixtureError> {
    let roots = trusted_roots(options)?;
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(MakeRustlsConnect::new(config))
}

fn trusted_roots(options: &ConnectionOptions) -> Result<RootCertStore, FixtureError> {
    let mut roots = RootCertStore::empty();
    if let Some(path) = &options.root_certificate {
        let file = File::open(path).map_err(FixtureError::TlsIo)?;
        let mut reader = BufReader::new(file);
        for certificate_result in rustls_pemfile::certs(&mut reader) {
            let certificate = certificate_result.map_err(FixtureError::TlsIo)?;
            roots
                .add(certificate)
                .map_err(FixtureError::TlsCertificate)?;
        }
    } else {
        let native = rustls_native_certs::load_native_certs();
        for certificate in native.certs {
            roots
                .add(certificate)
                .map_err(FixtureError::TlsCertificate)?;
        }
    }
    if roots.is_empty() {
        return Err(FixtureError::TlsNoRoots);
    }
    Ok(roots)
}
