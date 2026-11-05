#![cfg(test)]

use super::validate_environment;
use crate::aws_kms::AwsKmsError;
use std::collections::BTreeMap;

#[test]
fn validate_environment_rejects_missing_aws_region_name() {
    assert!(matches!(
        validate_environment(&BTreeMap::new()),
        Err(AwsKmsError::MissingRegion)
    ));
}
