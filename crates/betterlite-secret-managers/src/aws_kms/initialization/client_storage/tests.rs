#![cfg(test)]

use aws_smithy_http_client::{
    Builder,
    tls::{Provider, rustls_provider::CryptoMode},
};

use std::sync::Arc;

use super::{build_client, store_client};
use crate::aws_kms::AWS_KMS_CLIENT;

#[tokio::test]
async fn client_storage_retains_the_built_client() {
    let transport = Builder::new()
        .tls_provider(Provider::Rustls(CryptoMode::AwsLc))
        .build_https();
    let client = build_client("us-east-1".to_owned(), transport, None);
    store_client(Arc::clone(&client)).unwrap();
    assert!(AWS_KMS_CLIENT.lock().unwrap().is_some());
}
