#![cfg(test)]

use std::sync::Arc;

use super::{build_client, store_client};
use crate::aws_kms::AWS_KMS_CLIENT;

#[tokio::test]
async fn client_storage_retains_the_built_client() {
    let client = build_client("us-east-1".to_owned()).await;
    store_client(Arc::clone(&client)).unwrap();
    assert!(AWS_KMS_CLIENT.lock().unwrap().is_some());
}
