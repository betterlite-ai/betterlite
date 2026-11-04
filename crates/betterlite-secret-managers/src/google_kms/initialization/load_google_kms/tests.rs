#![cfg(test)]

use super::load_google_kms;

#[tokio::test]
async fn load_google_kms_does_not_initialize_when_disabled() {
    load_google_kms(Some(false), std::sync::Arc::new(super::tests::Transport)).unwrap();
}

#[tokio::test]
async fn load_google_kms_keeps_the_injected_transport() {
    let client = crate::google_kms::Client {
        resource_name: "resource".to_owned(),
        transport: std::sync::Arc::new(super::tests::Transport),
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
