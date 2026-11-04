#![cfg(test)]

use std::sync::Arc;

use super::{build_client, store_client};
use crate::aws_kms::AWS_KMS_CLIENT;

#[tokio::test]
async fn client_storage_retains_the_built_client() {
    let transport = aws_smithy_http_client::Builder::new()
        .tls_provider(aws_smithy_http_client::tls::Provider::Rustls(
            aws_smithy_http_client::tls::rustls_provider::CryptoMode::AwsLc,
        ))
        .build_https();
    let client = build_client("us-east-1".to_owned(), transport, None);
    store_client(Arc::clone(&client)).unwrap();
    assert!(AWS_KMS_CLIENT.lock().unwrap().is_some());
}
