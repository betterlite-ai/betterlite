#![cfg(test)]

use super::load_aws_kms;
use crate::aws_kms::{
    AWS_KMS_CLIENT,
    tests::{credentials, serve, transport},
};
use aws_sdk_kms::primitives::Blob;
use rstest::rstest;
use std::{collections::BTreeMap, time::Duration};
use tokio::net::TcpListener;

#[test]
fn load_aws_kms_does_not_initialize_when_disabled() {
    load_aws_kms(
        Some(false),
        &BTreeMap::new(),
        transport(),
        None,
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(AWS_KMS_CLIENT.lock().unwrap().is_none());
}

#[test]
fn load_aws_kms_builds_a_client_for_the_configured_region() {
    let environment = BTreeMap::from([("AWS_REGION_NAME".to_owned(), "us-west-2".to_owned())]);
    load_aws_kms(
        Some(true),
        &environment,
        transport(),
        None,
        Duration::from_secs(3),
    )
    .unwrap();
    let config = AWS_KMS_CLIENT
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .config()
        .clone();
    assert_eq!(config.region().unwrap().as_ref(), "us-west-2");
    assert_eq!(
        config.timeout_config().unwrap().operation_timeout(),
        Some(Duration::from_secs(3))
    );
}

#[rstest]
#[case::service_over_global("AWS_ENDPOINT_URL_KMS", "AWS_ENDPOINT_URL", "http://127.0.0.1:1")]
#[case::global_when_service_empty("AWS_ENDPOINT_URL", "AWS_ENDPOINT_URL_KMS", "")]
#[tokio::test]
async fn load_aws_kms_sends_signed_decrypt_calls_to_the_configured_endpoint(
    #[case] endpoint_name: &str,
    #[case] other_name: &str,
    #[case] other_value: &str,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let environment = BTreeMap::from([
        (endpoint_name.to_owned(), format!("http://{address}")),
        (other_name.to_owned(), other_value.to_owned()),
        ("AWS_REGION_NAME".to_owned(), "us-west-2".to_owned()),
    ]);
    load_aws_kms(
        Some(true),
        &environment,
        transport(),
        credentials(),
        Duration::from_secs(5),
    )
    .unwrap();
    let client = AWS_KMS_CLIENT.lock().unwrap().clone().unwrap();
    let (response, request) = tokio::join!(
        client
            .decrypt()
            .ciphertext_blob(Blob::new(b"hello".to_vec()))
            .send(),
        serve(&listener, 200, r#"{"Plaintext":"aGk="}"#)
    );
    assert_eq!(response.unwrap().plaintext().unwrap().as_ref(), b"hi");
    let headers = request.to_lowercase();
    assert!(request.starts_with("POST / HTTP/1.1\r\n"), "{request}");
    assert!(
        headers.contains("x-amz-target: trentservice.decrypt\r\n"),
        "{request}"
    );
    assert!(headers.contains("/us-west-2/kms/aws4_request"), "{request}");
}
