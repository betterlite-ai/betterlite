#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::billing_metrics::Metering;
use rcgen::{CertificateParams, KeyPair};
use tempfile::TempDir;

use super::{TlsError, certificates, tls_config};

struct Identity {
    certificate_pem: String,
    key_pem: String,
}

fn self_signed(common_name: &str) -> Identity {
    let key = KeyPair::generate().unwrap();
    let params = CertificateParams::new(vec![common_name.to_owned()]).unwrap();
    let certificate = params.self_signed(&key).unwrap();
    Identity {
        certificate_pem: certificate.pem(),
        key_pem: key.serialize_pem(),
    }
}

fn write(directory: &TempDir, name: &str, contents: &str) -> PathBuf {
    let path = directory.path().join(name);
    fs::write(&path, contents).unwrap();
    path
}

fn metering(
    directory: &TempDir,
    client_cert: &str,
    client_key: &str,
    ca_cert: Option<&str>,
) -> Metering {
    Metering {
        endpoint: "https://collector.example/v1/metrics".to_owned(),
        client_cert: write(directory, "client.crt", client_cert),
        client_key: write(directory, "client.key", client_key),
        ca_cert: ca_cert.map(|pem| write(directory, "ca.crt", pem)),
        export_interval_ms: 60_000,
    }
}

#[test]
fn builds_from_a_valid_client_certificate_and_a_custom_authority() {
    let directory = TempDir::new().unwrap();
    let client = self_signed("billing-client");
    let authority = self_signed("billing-ca");
    let settings = metering(
        &directory,
        &client.certificate_pem,
        &client.key_pem,
        Some(&authority.certificate_pem),
    );
    tls_config(&settings).unwrap();
}

#[test]
fn rejects_a_client_certificate_file_with_no_certificate() {
    let directory = TempDir::new().unwrap();
    let client = self_signed("billing-client");
    let authority = self_signed("billing-ca");
    let settings = metering(
        &directory,
        "",
        &client.key_pem,
        Some(&authority.certificate_pem),
    );
    let error = tls_config(&settings).unwrap_err();
    assert!(matches!(error, TlsError::NoCertificate { .. }), "{error}");
}

#[test]
fn rejects_a_custom_authority_file_with_no_certificate() {
    let directory = TempDir::new().unwrap();
    let client = self_signed("billing-client");
    let settings = metering(
        &directory,
        &client.certificate_pem,
        &client.key_pem,
        Some(""),
    );
    let error = tls_config(&settings).unwrap_err();
    assert!(matches!(error, TlsError::NoTrustRoots), "{error}");
}

#[test]
fn rejects_a_client_key_that_does_not_match_the_certificate() {
    let directory = TempDir::new().unwrap();
    let client = self_signed("billing-client");
    let other = self_signed("someone-else");
    let authority = self_signed("billing-ca");
    let settings = metering(
        &directory,
        &client.certificate_pem,
        &other.key_pem,
        Some(&authority.certificate_pem),
    );
    let error = tls_config(&settings).unwrap_err();
    assert!(matches!(error, TlsError::Tls { .. }), "{error}");
}

#[test]
fn certificates_reads_every_certificate_in_a_pem_file() {
    let directory = TempDir::new().unwrap();
    let first = self_signed("first");
    let second = self_signed("second");
    let bundle = format!("{}{}", first.certificate_pem, second.certificate_pem);
    let path = write(&directory, "bundle.crt", &bundle);
    let parsed = certificates(&path).unwrap();
    assert_eq!(parsed.len(), 2);
}

#[test]
fn certificates_rejects_an_unreadable_path() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("missing.crt");
    let error = certificates(&path).unwrap_err();
    assert!(matches!(error, TlsError::Pem { .. }), "{error}");
}
