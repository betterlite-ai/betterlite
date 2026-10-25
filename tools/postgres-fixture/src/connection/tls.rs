use rustls::{
    ClientConfig, RootCertStore,
    pki_types::{CertificateDer, pem::PemObject as _},
};
use tokio_postgres_rustls_improved::MakeRustlsConnect;
use url::Url;

use super::ConnectionOptions;
use crate::error::FixtureError;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    Disable,
    Require,
    VerifyCa,
    VerifyFull,
}

pub(super) fn tls_mode(raw: &str) -> Result<TlsMode, FixtureError> {
    match query_value(raw, "sslmode").as_deref() {
        None | Some("disable") => Ok(TlsMode::Disable),
        Some("prefer") => Err(FixtureError::ExplicitPreferTls),
        Some("require") => Ok(TlsMode::Require),
        Some("verify-ca") => Ok(TlsMode::VerifyCa),
        Some("verify-full") => Ok(TlsMode::VerifyFull),
        Some(value) => Err(FixtureError::UnknownTlsMode {
            value: value.to_owned(),
        }),
    }
}

pub(super) fn query_value(raw: &str, key: &str) -> Option<String> {
    let parsed = Url::parse(raw).ok()?;
    parsed
        .query_pairs()
        .find_map(|(name, value)| (name == key).then(|| value.into_owned()))
}

pub(super) fn normalized_config_source(raw: &str, tls_mode: TlsMode) -> String {
    let Ok(mut parsed) = Url::parse(raw) else {
        return raw.to_owned();
    };
    let had_query = parsed.query().is_some();
    let parameters = parsed
        .query_pairs()
        .filter_map(|(name, configured_value)| {
            if name == "sslrootcert" {
                return None;
            }
            let normalized_value = if name == "sslmode" && tls_mode != TlsMode::Disable {
                "require".to_owned()
            } else {
                configured_value.into_owned()
            };
            Some((name.into_owned(), normalized_value))
        })
        .collect::<Vec<_>>();
    parsed.query_pairs_mut().clear().extend_pairs(parameters);
    if had_query {
        parsed.to_string()
    } else {
        raw.to_owned()
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
        let certificates = CertificateDer::pem_file_iter(path).map_err(FixtureError::TlsPem)?;
        for certificate_result in certificates {
            let certificate = certificate_result.map_err(FixtureError::TlsPem)?;
            roots
                .add(certificate)
                .map_err(FixtureError::TlsCertificate)?;
        }
    } else {
        let native = rustls_native_certs::load_native_certs();
        if let Some(error) = native.errors.into_iter().next() {
            return Err(FixtureError::TlsNative(error));
        }
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
