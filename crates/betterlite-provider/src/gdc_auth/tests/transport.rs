use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use crate::gdc_auth::transport::{CaCertificates as _, CaError, FileCaCertificates, MAX_CA_BYTES};

fn temp_path(name: &str) -> PathBuf {
    env::temp_dir().join(format!("betterlite-gdc-ca-{}-{name}.pem", process::id()))
}

#[tokio::test]
async fn a_ca_file_is_read_verbatim() {
    let path = temp_path("verbatim");
    fs::write(&path, b"-----BEGIN CERTIFICATE-----").unwrap();
    let loaded = FileCaCertificates.load(path.to_str().unwrap()).await;
    fs::remove_file(&path).unwrap();
    assert_eq!(loaded.unwrap(), b"-----BEGIN CERTIFICATE-----");
}

#[tokio::test]
async fn a_missing_ca_file_names_its_path() {
    let path = temp_path("missing");
    let error = FileCaCertificates
        .load(path.to_str().unwrap())
        .await
        .unwrap_err();
    assert!(matches!(&error, CaError::Read { path: named, .. } if named == path.to_str().unwrap()));
}

#[tokio::test]
async fn the_ca_size_limit_keeps_the_boundary_and_refuses_one_byte_more() {
    let limit = usize::try_from(MAX_CA_BYTES).unwrap();
    let path = temp_path("limit");
    fs::write(&path, vec![b'a'; limit]).unwrap();
    let at_limit = FileCaCertificates.load(path.to_str().unwrap()).await;
    fs::write(&path, vec![b'a'; limit + 1]).unwrap();
    let over_limit = FileCaCertificates.load(path.to_str().unwrap()).await;
    fs::remove_file(&path).unwrap();
    assert_eq!(at_limit.unwrap().len(), limit);
    assert!(matches!(
        over_limit.unwrap_err(),
        CaError::TooLarge { limit: reported, .. } if reported == MAX_CA_BYTES
    ));
}
