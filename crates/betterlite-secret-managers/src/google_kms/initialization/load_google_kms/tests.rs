#![cfg(test)]

use super::{build_client, load_google_kms};

#[tokio::test]
async fn load_google_kms_does_not_initialize_when_disabled() {
    load_google_kms(Some(false)).await.unwrap();
}

#[tokio::test]
async fn load_google_kms_constructs_the_google_sdk_client() {
    let _client = build_client().await.unwrap();
}
