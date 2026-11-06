#![cfg(test)]

use secrecy::{ExposeSecret as _, SecretString};

use super::delete;
use crate::cyberark::cyberark_rotate::rotate;
use crate::cyberark::operation::OperationStatus;
use crate::cyberark::tests::{POLICY_ROUTE, Reply, capture_logs, keyed};
use crate::secret_contracts::options::{DeleteOptions, SecretOptions, SecretRotation, SecretWrite};
use crate::secret_contracts::{BaseSecretManager as _, RotationError};

const OLD_READ: &str = "GET http://127.0.0.1:8080/secrets/default/variable/old";
const NEW_READ: &str = "GET http://127.0.0.1:8080/secrets/default/variable/new";
const NEW_WRITE: &str = "POST http://127.0.0.1:8080/secrets/default/variable/new";
const OLD_WRITE: &str = "POST http://127.0.0.1:8080/secrets/default/variable/old";

#[tokio::test(start_paused = true)]
async fn delete_is_not_supported_and_clears_the_cached_value() {
    let (logs, _guard) = capture_logs();
    let (manager, transport) = keyed(&[(OLD_READ, vec![Reply::Status(200, "v")])]);
    assert!(manager.read("old").await.is_some());
    let before = transport.recorded().len();
    let outcome = delete(&manager, "old");
    assert_eq!(outcome.status, OperationStatus::NotSupported);
    assert_eq!(
        outcome.message,
        "CyberArk Conjur does not support direct secret deletion. Use policy updates to remove variables."
    );
    assert_eq!(transport.recorded().len(), before);
    assert!(
        logs.text()
            .contains("WARN CyberArk Conjur does not support direct secret deletion.")
    );
    manager.read("old").await;
    assert_eq!(transport.recorded().len(), before + 1);
}

#[tokio::test(start_paused = true)]
async fn rotation_writes_the_new_secret_verifies_it_from_the_cache_and_forgets_the_old_one() {
    let (manager, transport) = keyed(&[
        (OLD_READ, vec![Reply::Status(200, "before")]),
        (POLICY_ROUTE, vec![Reply::Status(201, "")]),
        (NEW_WRITE, vec![Reply::Status(201, "")]),
    ]);
    let value = SecretString::from("after");
    let outcome = rotate(
        &manager,
        SecretRotation {
            current_secret_name: "old",
            new_secret_name: "new",
            new_secret_value: &value,
            options: SecretOptions::default(),
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.status, OperationStatus::Success);
    assert_eq!(outcome.message, "Secret new written successfully");
    let routes = transport.routes_hit();
    assert_eq!(routes[1..], [OLD_READ, POLICY_ROUTE, NEW_WRITE]);
    assert_eq!(manager.read("new").await.unwrap().expose_secret(), "after");
    assert_eq!(transport.routes_hit().len(), routes.len());
    manager.read("old").await;
    assert_eq!(
        transport.routes_hit().last().map(String::as_str),
        Some(OLD_READ)
    );
}

#[tokio::test(start_paused = true)]
async fn rotating_a_secret_onto_its_own_name_rewrites_and_clears_the_cache() {
    let (manager, transport) = keyed(&[
        (OLD_READ, vec![Reply::Status(200, "before")]),
        (POLICY_ROUTE, vec![Reply::Status(201, "")]),
        (OLD_WRITE, vec![Reply::Status(201, "")]),
    ]);
    let value = SecretString::from("after");
    let outcome = rotate(
        &manager,
        SecretRotation {
            current_secret_name: "old",
            new_secret_name: "old",
            new_secret_value: &value,
            options: SecretOptions::default(),
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.status, OperationStatus::Success);
    let routes = transport.routes_hit();
    assert_eq!(routes[1..], [OLD_READ, POLICY_ROUTE, OLD_WRITE]);
    manager.read("old").await;
    assert_eq!(
        transport.routes_hit().last().map(String::as_str),
        Some(OLD_READ)
    );
}

#[tokio::test(start_paused = true)]
async fn rotation_needs_the_current_secret() {
    let (manager, transport) = keyed(&[(OLD_READ, vec![Reply::Status(404, "")])]);
    let value = SecretString::from("after");
    let error = rotate(
        &manager,
        SecretRotation {
            current_secret_name: "old",
            new_secret_name: "new",
            new_secret_value: &value,
            options: SecretOptions::default(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(error, RotationError::CurrentSecretNotFound));
    assert!(!transport.routes_hit().contains(&POLICY_ROUTE.to_owned()));
}

#[tokio::test(start_paused = true)]
async fn rotation_fails_when_the_new_secret_cannot_be_verified() {
    let (manager, _transport) = keyed(&[
        (OLD_READ, vec![Reply::Status(200, "before")]),
        (POLICY_ROUTE, vec![Reply::Status(201, "")]),
        (NEW_WRITE, vec![Reply::Status(500, "")]),
        (NEW_READ, vec![Reply::Status(404, "")]),
    ]);
    let value = SecretString::from("after");
    let error = rotate(
        &manager,
        SecretRotation {
            current_secret_name: "old",
            new_secret_name: "new",
            new_secret_value: &value,
            options: SecretOptions::default(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(error, RotationError::NewSecretNotVerified));
    assert_eq!(manager.read("old").await.unwrap().expose_secret(), "before");
}

#[tokio::test(start_paused = true)]
async fn the_trait_methods_read_and_write() {
    let (manager, _transport) = keyed(&[
        (OLD_READ, vec![Reply::Status(200, "v")]),
        (POLICY_ROUTE, vec![Reply::Status(201, "")]),
        (OLD_WRITE, vec![Reply::Status(201, "")]),
    ]);
    let read = manager
        .async_read_secret("old", SecretOptions::default())
        .await
        .unwrap();
    assert_eq!(read.unwrap().expose_secret(), "v");
    let value = SecretString::from("w");
    let written = manager
        .async_write_secret(SecretWrite {
            secret_name: "old",
            secret_value: &value,
            description: None,
            options: SecretOptions::default(),
            tags: None,
        })
        .await
        .unwrap();
    assert_eq!(written.status, OperationStatus::Success);
}

#[tokio::test(start_paused = true)]
async fn the_trait_methods_delete_sync_read_and_rotate() {
    let (manager, transport) = keyed(&[
        (OLD_READ, vec![Reply::Status(200, "v")]),
        (POLICY_ROUTE, vec![Reply::Status(201, "")]),
        (OLD_WRITE, vec![Reply::Status(201, "")]),
    ]);
    let deleted = manager
        .async_delete_secret("old", DeleteOptions::default())
        .await
        .unwrap();
    assert_eq!(deleted.status, OperationStatus::NotSupported);
    let synced = manager
        .sync_read_secret("old", SecretOptions::default())
        .unwrap();
    assert_eq!(synced.unwrap().expose_secret(), "v");
    assert!(transport.recorded().last().unwrap().blocking);
    let value = SecretString::from("w");
    let rotated = manager
        .async_rotate_secret(SecretRotation {
            current_secret_name: "old",
            new_secret_name: "old",
            new_secret_value: &value,
            options: SecretOptions::default(),
        })
        .await
        .unwrap();
    assert_eq!(rotated.status, OperationStatus::Success);
}
