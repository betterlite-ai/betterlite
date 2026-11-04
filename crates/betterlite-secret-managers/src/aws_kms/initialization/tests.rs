#![cfg(test)]

use super::{build_client, load_aws_kms, store_client, validate_region};
use crate::aws_kms::{AWS_KMS_CLIENT, AwsKmsError};
use std::sync::Arc;

#[test]
fn validate_environment_rejects_missing_aws_region_name() {
    assert!(matches!(
        validate_region(None),
        Err(AwsKmsError::MissingRegion)
    ));
}

#[tokio::test]
async fn load_aws_kms_does_not_initialize_when_disabled() {
    load_aws_kms(Some(false)).await.unwrap();
}

#[tokio::test]
async fn load_aws_kms_builds_a_client_for_the_configured_region() {
    let client = build_client("us-east-1".to_owned()).await;
    assert_eq!(
        client.config().region().map(AsRef::as_ref),
        Some("us-east-1")
    );
}

#[tokio::test]
async fn client_storage_retains_the_built_client() {
    let client = build_client("us-east-1".to_owned()).await;
    store_client(Arc::clone(&client)).unwrap();
    assert!(AWS_KMS_CLIENT.lock().unwrap().is_some());
}
