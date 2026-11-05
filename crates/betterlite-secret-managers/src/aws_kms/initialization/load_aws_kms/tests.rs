#![cfg(test)]

use super::load_aws_kms;
use crate::aws_kms::{
    AWS_KMS_CLIENT,
    tests::{credentials, transport, with_kms},
};
use aws_sdk_kms::primitives::Blob;
use axum::http::{Method, header::AUTHORIZATION};
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
        Some(credentials()),
        Duration::from_secs(5),
    )
    .unwrap();
    let client = AWS_KMS_CLIENT.lock().unwrap().clone().unwrap();
    let decrypt = Box::pin(
        client
            .decrypt()
            .ciphertext_blob(Blob::new(b"hello".to_vec()))
            .send(),
    );
    let (response, requests) = with_kms(listener, 200, r#"{"Plaintext":"aGk="}"#, decrypt).await;
    assert_eq!(response.unwrap().plaintext().unwrap().as_ref(), b"hi");
    let [request] = requests.as_slice() else {
        panic!("expected one request");
    };
    assert_eq!(request.head.method, Method::POST);
    assert_eq!(request.head.uri, "/");
    assert_eq!(request.head.headers["x-amz-target"], "TrentService.Decrypt");
    assert!(
        request.head.headers[AUTHORIZATION]
            .to_str()
            .unwrap()
            .contains("/us-west-2/kms/aws4_request")
    );
}
