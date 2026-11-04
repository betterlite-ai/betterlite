#![cfg(test)]

use super::{load_aws_kms, validate_region};
use crate::aws_kms::{AWS_KMS_CLIENT, AwsKmsError};

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
    assert!(AWS_KMS_CLIENT.lock().unwrap().is_none());
}

#[test]
fn client_storage_is_a_process_wide_client_cell() {
    assert!(AWS_KMS_CLIENT.lock().unwrap().is_none());
}
