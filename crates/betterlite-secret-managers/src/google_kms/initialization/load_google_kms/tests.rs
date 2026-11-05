#![cfg(test)]

use super::tests::Transport;
use crate::google_kms::Client;
use std::sync::Arc;

use super::load_google_kms;

#[tokio::test]
async fn load_google_kms_does_not_initialize_when_disabled() {
    load_google_kms(Some(false), Arc::new(Transport)).unwrap();
}

#[tokio::test]
async fn load_google_kms_keeps_the_injected_transport() {
    let client = Client {
        resource_name: "resource".to_owned(),
        transport: Arc::new(Transport),
    };
    assert_eq!(
        client
            .transport
            .decrypt(&client.resource_name, b"ciphertext")
            .await
            .unwrap(),
        b"plaintext"
    );
}
