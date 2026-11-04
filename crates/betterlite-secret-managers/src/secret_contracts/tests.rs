#![cfg(test)]

#[path = "tests/manager.rs"]
mod manager;
use super::BaseSecretManager as _;
use super::{SecretContractError, SecretOperationOptions, SecretRotation};
use manager::FakeManager;

#[tokio::test]
async fn rotation_creates_verifies_then_deletes_with_recovery_window() {
    let manager = FakeManager::default();
    let response = manager
        .async_rotate_secret(SecretRotation {
            current_secret_name: "old",
            new_secret_name: "new",
            new_secret_value: "new-value",
            options: SecretOperationOptions {
                optional_params: None,
                timeout: None,
            },
        })
        .await;
    let create_response = response.expect("rotation should return the create response");
    assert_eq!(
        create_response,
        super::SecretResponse::from([(
            String::from("name"),
            super::SecretValue::String(String::from("new")),
        )])
    );
    assert_eq!(
        manager.calls.lock().ok().map(|calls| calls.clone()),
        Some(vec![
            String::from("read:old"),
            String::from("write:new"),
            String::from("read:new"),
            String::from("delete:old:7"),
        ])
    );
}

#[tokio::test]
async fn rotation_does_not_write_when_current_secret_is_missing() {
    let manager = FakeManager::default();
    if let Ok(mut missing) = manager.missing.lock() {
        missing.push(String::from("old"));
    }
    let result = manager
        .async_rotate_secret(SecretRotation {
            current_secret_name: "old",
            new_secret_name: "new",
            new_secret_value: "value",
            options: SecretOperationOptions {
                optional_params: None,
                timeout: None,
            },
        })
        .await;
    assert!(matches!(
        result,
        Err(SecretContractError::CurrentSecretNotFound { .. })
    ));
    assert_eq!(
        manager.calls.lock().ok().map(|calls| calls.clone()),
        Some(vec![String::from("read:old")])
    );
}

#[tokio::test]
async fn rotation_does_not_delete_when_new_secret_cannot_be_verified() {
    let manager = FakeManager::default();
    if let Ok(mut missing) = manager.missing.lock() {
        missing.push(String::from("new"));
    }
    let result = manager
        .async_rotate_secret(SecretRotation {
            current_secret_name: "old",
            new_secret_name: "new",
            new_secret_value: "value",
            options: SecretOperationOptions {
                optional_params: None,
                timeout: None,
            },
        })
        .await;
    assert!(matches!(
        result,
        Err(SecretContractError::NewSecretNotVerified { .. })
    ));
    assert_eq!(
        manager.calls.lock().ok().map(|calls| calls.clone()),
        Some(vec![
            String::from("read:old"),
            String::from("write:new"),
            String::from("read:new")
        ])
    );
}
