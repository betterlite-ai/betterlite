#![cfg(test)]

mod manager;

use std::error::Error as _;
use std::sync::Mutex;
use std::time::Duration;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::options::{
    DeleteOptions, SecretOptions, SecretRotation, SecretTags, SecretTimeout, SecretWrite,
};
use super::{BaseSecretManager as _, RotationError};
use manager::{FakeManager, Operation, Params, TestError};

#[tokio::test]
async fn rotation_creates_verifies_then_deletes_with_recovery_window() {
    let manager = FakeManager::default();
    let params = Params { version: 3 };
    let timeout = SecretTimeout::Total(Duration::from_secs(12));
    let response = manager
        .async_rotate_secret(SecretRotation {
            current_secret_name: "old",
            new_secret_name: "new",
            new_secret_value: &SecretString::from("new-value"),
            options: SecretOptions {
                optional_params: Some(&params),
                timeout: Some(timeout),
            },
        })
        .await
        .unwrap();
    assert_eq!(response, "created");
    let calls = manager.calls.into_inner().unwrap();
    assert_eq!(calls.len(), 4);
    assert_eq!(
        calls
            .iter()
            .map(|call| call.name.as_str())
            .collect::<Vec<_>>(),
        ["old", "new", "new", "old"]
    );
    assert!(
        calls
            .iter()
            .all(|call| call.params == Some(params) && call.timeout == Some(timeout))
    );
    assert!(matches!(calls[0].operation, Operation::Read));
    assert_rotation_write(&calls[1].operation);
    assert!(matches!(calls[2].operation, Operation::Read));
    assert!(matches!(calls[3].operation, Operation::Delete(7)));
}

#[rstest]
#[case::old_missing("old", 1)]
#[case::new_missing("new", 3)]
#[tokio::test]
async fn rotation_stops_on_missing_secrets(#[case] missing: &'static str, #[case] count: usize) {
    let manager = FakeManager {
        missing: Some(missing),
        calls: Mutex::new(Vec::new()),
        fail_at: None,
    };
    let error = manager
        .async_rotate_secret(SecretRotation {
            current_secret_name: "old",
            new_secret_name: "new",
            new_secret_value: &SecretString::from("new-value"),
            options: SecretOptions::default(),
        })
        .await
        .unwrap_err();
    match missing {
        "old" => assert!(matches!(error, RotationError::CurrentSecretNotFound)),
        "new" => assert!(matches!(error, RotationError::NewSecretNotVerified)),
        _ => panic!("unexpected fixture"),
    }
    assert_eq!(manager.calls.lock().unwrap().len(), count);
}

#[rstest]
#[case::read_old(1, TestError::Timeout)]
#[case::write(2, TestError::HttpStatus)]
#[case::verify(3, TestError::Timeout)]
#[case::delete(4, TestError::HttpStatus)]
#[tokio::test]
async fn rotation_stops_at_failure_and_keeps_cause(#[case] step: usize, #[case] cause: TestError) {
    let manager = FakeManager {
        fail_at: Some((step, cause.clone())),
        calls: Mutex::new(Vec::new()),
        missing: None,
    };
    let error = manager
        .async_rotate_secret(SecretRotation {
            current_secret_name: "old",
            new_secret_name: "new",
            new_secret_value: &SecretString::from("new-value"),
            options: SecretOptions::default(),
        })
        .await
        .unwrap_err();
    assert_eq!(
        error.source().unwrap().downcast_ref::<TestError>(),
        Some(&cause)
    );
    assert_eq!(error.to_string(), "secret operation failed");
    assert_eq!(manager.calls.lock().unwrap().len(), step);
}

#[tokio::test]
async fn rotation_rejects_identical_names_without_modifying_secrets() {
    let manager = FakeManager::default();
    let error = manager
        .async_rotate_secret(SecretRotation {
            current_secret_name: "old",
            new_secret_name: "old",
            new_secret_value: &SecretString::from("new-value"),
            options: SecretOptions::default(),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, RotationError::SameName));
    assert!(manager.calls.lock().unwrap().is_empty());
}

#[test]
fn rotation_debug_redacts_the_new_secret() {
    let value = SecretString::from("private-value");
    let rotation = SecretRotation::<()> {
        current_secret_name: "old",
        new_secret_name: "new",
        new_secret_value: &value,
        options: SecretOptions::default(),
    };
    let write = SecretWrite::<()> {
        secret_name: "new",
        secret_value: &value,
        description: None,
        options: SecretOptions::default(),
        tags: None,
    };
    assert!(!format!("{rotation:?} {write:?}").contains("private-value"));
}

#[rstest]
#[case::mapping(r#"{"Environment":"Prod"}"#)]
#[case::list(r#"[{"Key":"Environment","Value":"Prod"}]"#)]
#[tokio::test]
async fn write_accepts_tag_shapes_and_phase_timeouts(#[case] json: &str) {
    let tags: SecretTags = serde_json::from_str(json).unwrap();
    assert_eq!(serde_json::to_string(&tags).unwrap(), json);
    let timeout = SecretTimeout::Phases {
        connect: Some(Duration::ZERO),
        read: None,
        write: Some(Duration::from_secs(3)),
        pool: Some(Duration::from_secs(4)),
    };
    let manager = FakeManager::default();
    let response = manager
        .async_write_secret(SecretWrite {
            secret_name: "name",
            secret_value: &SecretString::from("private-value"),
            description: Some("description"),
            options: SecretOptions {
                optional_params: None,
                timeout: Some(timeout),
            },
            tags: Some(&tags),
        })
        .await
        .unwrap();
    assert_eq!(response, "created");
    let calls = manager.calls.into_inner().unwrap();
    assert_eq!(calls[0].timeout, Some(timeout));
    let Operation::Write {
        tags: received,
        description,
        value,
    } = &calls[0].operation
    else {
        panic!("expected write")
    };
    assert_eq!(received.as_ref(), Some(&tags));
    assert_eq!(description.as_deref(), Some("description"));
    assert_eq!(value.expose_secret(), "private-value");
}

#[tokio::test]
async fn delete_defaults_to_seven_days_and_accepts_an_override() {
    let manager = FakeManager::default();
    manager
        .async_delete_secret("old", DeleteOptions::default())
        .await
        .unwrap();
    manager
        .async_delete_secret(
            "old",
            DeleteOptions {
                recovery_window_in_days: 30,
                operation: SecretOptions::default(),
            },
        )
        .await
        .unwrap();
    let calls = manager.calls.into_inner().unwrap();
    assert!(matches!(calls[0].operation, Operation::Delete(7)));
    assert!(matches!(calls[1].operation, Operation::Delete(30)));
}

fn assert_rotation_write(operation: &Operation) {
    let Operation::Write {
        value,
        description,
        tags,
    } = operation
    else {
        panic!("expected write")
    };
    assert_eq!(value.expose_secret(), "new-value");
    assert_eq!(description.as_deref(), Some("Rotated from old"));
    assert!(tags.is_none());
}
