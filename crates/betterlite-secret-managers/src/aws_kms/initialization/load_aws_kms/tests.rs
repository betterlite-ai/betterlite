#![cfg(test)]

use super::{build_client, load_aws_kms};

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
