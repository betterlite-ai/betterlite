#![cfg(test)]

use super::validate_region;
use crate::aws_kms::AwsKmsError;

#[test]
fn validate_environment_rejects_missing_aws_region_name() {
    assert!(matches!(
        validate_region(None),
        Err(AwsKmsError::MissingRegion)
    ));
}
