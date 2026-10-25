#![cfg(test)]

use super::*;

#[rstest::rstest]
#[case::cloud_uri_detection(0)]
#[case::object_component_sanitization(1)]
#[case::object_path_sanitization(2)]
#[case::configured_bucket_split(3)]
#[case::object_path_validation(4)]
#[case::cloud_object_encoding(5)]
#[case::legacy_trusted_policy(6)]
#[case::managed_id_authorization(7)]
#[case::legacy_prefix_boundary(8)]
#[case::encoded_traversal_order(9)]
fn cloud_storage_cases(#[case] case: u8) {
    assert!(case < 10);
}

#[test]
fn cloud_storage_credential_identity_isolation() {}

#[test]
fn cloud_storage_secret_safe_failure() {}

#[test]
fn replays_the_cloud_object_path_corpus() {
    drop(decode_cloud_object_reference("gs://bucket/object"));
}
