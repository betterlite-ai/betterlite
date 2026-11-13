use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::cyberark::operation::{OperationStatus, SecretOperation};
use crate::secret_contracts::BaseSecretManager as _;
use crate::secret_contracts::options::{
    DeleteOptions, SecretOptions, SecretRotation, SecretTimeout, SecretWrite,
};
use crate::vault::error::VaultResponseError;
use crate::vault::target::{VaultSecretParams, VaultSecretSettings};
use crate::vault::transport::{MAX_RESPONSE_BYTES, VaultMethod};
use crate::vault_secret_operations::response::vault_secret_read_body;
use crate::vault_secret_operations::result::VaultOperationResult;

use super::support::{Reply, answered, capture_logs, scripted, slow};

const SECRET_BODY: &str = concat!(
    r#"{"data":{"data":{"key":"sk-from-vault","password":"pw-from-vault"},"#,
    r#""metadata":{"version":1}}}"#
);
const NEW_SECRET_BODY: &str = r#"{"data":{"data":{"key":"new-secret"}}}"#;
const OTHER_SECRET_BODY: &str = r#"{"data":{"data":{"key":"other-secret"}}}"#;
const WRITE_BODY: &str = r#"{"request_id":"req-1","data":{"version":1}}"#;
const READ_URL: &str = "GET http://127.0.0.1:8200/v1/secret/data/APP_KEY";
const OLD_URL: &str = "http://127.0.0.1:8200/v1/secret/data/OLD_KEY";
const NEW_URL: &str = "http://127.0.0.1:8200/v1/secret/data/NEW_KEY";
const OLD_GET: &str = "GET http://127.0.0.1:8200/v1/secret/data/OLD_KEY";
const NEW_GET: &str = "GET http://127.0.0.1:8200/v1/secret/data/NEW_KEY";
const NEW_POST: &str = "POST http://127.0.0.1:8200/v1/secret/data/NEW_KEY";
const OLD_DELETE: &str = "DELETE http://127.0.0.1:8200/v1/secret/data/OLD_KEY";

fn token_environment() -> Vec<(&'static str, &'static str)> {
    vec![("HCP_VAULT_TOKEN", "token")]
}

fn params_for(data_key: &str) -> VaultSecretParams {
    VaultSecretParams {
        secret_manager_settings: Some(VaultSecretSettings {
            namespace: None,
            mount: None,
            path_prefix: None,
            data: Some(data_key.to_owned()),
        }),
        flat: VaultSecretSettings {
            namespace: None,
            mount: None,
            path_prefix: None,
            data: None,
        },
    }
}

fn read_options(
    params: Option<&VaultSecretParams>,
    timeout: Option<SecretTimeout>,
) -> SecretOptions<'_, VaultSecretParams> {
    SecretOptions {
        optional_params: params,
        timeout,
    }
}

fn write_request<'borrowed>(
    name: &'borrowed str,
    value: &'borrowed SecretString,
    description: Option<&'borrowed str>,
    params: Option<&'borrowed VaultSecretParams>,
    timeout: Option<SecretTimeout>,
) -> SecretWrite<'borrowed, VaultSecretParams> {
    SecretWrite {
        secret_name: name,
        secret_value: value,
        description,
        options: SecretOptions {
            optional_params: params,
            timeout,
        },
        tags: None,
    }
}

fn rotation<'borrowed>(
    current: &'borrowed str,
    new: &'borrowed str,
    value: &'borrowed SecretString,
    timeout: Option<SecretTimeout>,
) -> SecretRotation<'borrowed, VaultSecretParams> {
    SecretRotation {
        current_secret_name: current,
        new_secret_name: new,
        new_secret_value: value,
        options: SecretOptions {
            optional_params: None,
            timeout,
        },
    }
}

fn sent_body(recorded: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    serde_json::from_str(recorded).expect("the write body is a nested map of text")
}

fn delete_request() -> DeleteOptions<'static, VaultSecretParams> {
    DeleteOptions {
        recovery_window_in_days: 7,
        operation: SecretOptions {
            optional_params: None,
            timeout: None,
        },
    }
}

fn status_of(result: VaultOperationResult) -> SecretOperation {
    match result {
        VaultOperationResult::Status(status) => status,
        VaultOperationResult::Response(_) => panic!("expected a status dict"),
    }
}

#[tokio::test]
async fn reading_a_secret_returns_the_value_at_the_default_data_key() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[(READ_URL, vec![answered(200, SECRET_BODY)])],
    );
    let value = manager
        .async_read_secret("APP_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(value.unwrap().expose_secret(), "sk-from-vault");
    let requests = transport.recorded();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, VaultMethod::Get);
    assert_eq!(requests[0].token.as_deref(), Some("token"));
    assert_eq!(requests[0].namespace, None);
    assert_eq!(requests[0].timeout, Duration::from_secs(600));
    assert!(requests[0].follow_redirects);
    assert_eq!(requests[0].max_response_bytes, MAX_RESPONSE_BYTES);
}

#[tokio::test]
async fn reading_a_secret_returns_the_value_at_a_custom_data_key() {
    let (manager, _transport) = scripted(
        &token_environment(),
        &[(READ_URL, vec![answered(200, SECRET_BODY)])],
    );
    let params = params_for("password");
    let value = manager
        .async_read_secret("APP_KEY", read_options(Some(&params), None))
        .await
        .unwrap();
    assert_eq!(value.unwrap().expose_secret(), "pw-from-vault");
}

#[tokio::test]
async fn reading_a_secret_uses_the_override_namespace_and_mount() {
    let route = "GET http://127.0.0.1:8200/v1/teams/team-b/kv-prod/data/virtual-keys/APP_KEY";
    let (manager, transport) = scripted(
        &token_environment(),
        &[(route, vec![answered(200, SECRET_BODY)])],
    );
    let params = VaultSecretParams {
        secret_manager_settings: Some(VaultSecretSettings {
            namespace: Some("teams/team-b".to_owned()),
            mount: Some("kv-prod".to_owned()),
            path_prefix: Some("virtual-keys".to_owned()),
            data: None,
        }),
        flat: VaultSecretSettings {
            namespace: None,
            mount: None,
            path_prefix: None,
            data: None,
        },
    };
    let value = manager
        .async_read_secret("APP_KEY", read_options(Some(&params), None))
        .await
        .unwrap();
    assert_eq!(value.unwrap().expose_secret(), "sk-from-vault");
    assert_eq!(transport.recorded().len(), 1);
}

#[tokio::test]
async fn a_second_read_returns_the_cached_body_for_another_data_key() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[(READ_URL, vec![answered(200, SECRET_BODY)])],
    );
    let password = params_for("password");
    let first = manager
        .async_read_secret("APP_KEY", read_options(None, None))
        .await
        .unwrap();
    let second = manager
        .async_read_secret("APP_KEY", read_options(Some(&password), None))
        .await
        .unwrap();
    let third = manager
        .async_read_secret("APP_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(first.unwrap().expose_secret(), "sk-from-vault");
    assert_eq!(second.unwrap().expose_secret(), "pw-from-vault");
    assert_eq!(third.unwrap().expose_secret(), "sk-from-vault");
    assert_eq!(transport.recorded().len(), 1);
}

#[tokio::test]
async fn the_sync_read_caches_the_body_the_async_read_reuses() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[(READ_URL, vec![answered(200, SECRET_BODY)])],
    );
    let synced = manager
        .sync_read_secret("APP_KEY", read_options(None, None))
        .unwrap();
    assert_eq!(synced.unwrap().expose_secret(), "sk-from-vault");
    let cached = manager
        .async_read_secret("APP_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(cached.unwrap().expose_secret(), "sk-from-vault");
    let requests = transport.recorded();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].blocking);
}

#[rstest]
#[case::http_error(answered(500, ""))]
#[case::transport_failure(Reply::Failure)]
#[case::not_json(answered(200, "not a body"))]
#[case::json_array(answered(200, "[]"))]
#[case::missing_value(answered(200, r#"{"data":{"data":{"other":"x"}}}"#))]
#[case::value_is_not_text(answered(200, r#"{"data":{"data":{"key":7}}}"#))]
#[tokio::test]
async fn a_failed_read_returns_none(#[case] reply: Reply) {
    let (manager, _transport) = scripted(&token_environment(), &[(READ_URL, vec![reply])]);
    let value = manager
        .async_read_secret("APP_KEY", read_options(None, None))
        .await
        .unwrap();
    assert!(value.is_none());
}

#[tokio::test]
async fn a_read_of_an_unsafe_name_returns_none_without_a_request() {
    let (manager, transport) = scripted(&token_environment(), &[]);
    let value = manager
        .async_read_secret("../escape", read_options(None, None))
        .await
        .unwrap();
    assert!(value.is_none());
    assert!(transport.recorded().is_empty());
}

#[tokio::test]
async fn a_read_keeps_the_default_timeout_even_when_one_is_given() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[(READ_URL, vec![answered(200, SECRET_BODY)])],
    );
    let timeout = Some(SecretTimeout::Total(Duration::from_secs(1)));
    let value = manager
        .async_read_secret("APP_KEY", read_options(None, timeout))
        .await
        .unwrap();
    assert_eq!(value.unwrap().expose_secret(), "sk-from-vault");
    assert_eq!(transport.recorded()[0].timeout, Duration::from_secs(600));
}

#[tokio::test]
async fn writing_a_secret_posts_the_nested_data_with_the_description() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[(
            "POST http://127.0.0.1:8200/v1/secret/data/APP_KEY",
            vec![answered(200, WRITE_BODY)],
        )],
    );
    let value = SecretString::from("new-value");
    let result = manager
        .async_write_secret(write_request(
            "APP_KEY",
            &value,
            Some("the key"),
            None,
            None,
        ))
        .await
        .unwrap();
    let VaultOperationResult::Response(response) = result else {
        panic!("a successful write returns the Vault response");
    };
    assert_eq!(response.body.expose_secret(), WRITE_BODY);
    let requests = transport.recorded();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, VaultMethod::Post);
    assert_eq!(requests[0].token.as_deref(), Some("token"));
    assert_eq!(requests[0].timeout, Duration::from_secs(600));
    let body = sent_body(requests[0].body.as_deref().unwrap());
    assert_eq!(body["data"]["key"], "new-value");
    assert_eq!(body["data"]["description"], "the key");
}

#[tokio::test]
async fn a_write_without_a_description_sends_only_the_value() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[(
            "POST http://127.0.0.1:8200/v1/secret/data/APP_KEY",
            vec![answered(200, WRITE_BODY)],
        )],
    );
    let value = SecretString::from("new-value");
    let result = manager
        .async_write_secret(write_request("APP_KEY", &value, None, None, None))
        .await
        .unwrap();
    assert!(matches!(result, VaultOperationResult::Response(_)));
    let body = sent_body(transport.recorded()[0].body.as_deref().unwrap());
    assert_eq!(body["data"]["key"], "new-value");
    assert!(!body["data"].contains_key("description"));
}

#[tokio::test]
async fn a_write_keeps_an_empty_description() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[(
            "POST http://127.0.0.1:8200/v1/secret/data/APP_KEY",
            vec![answered(200, WRITE_BODY)],
        )],
    );
    let value = SecretString::from("new-value");
    let result = manager
        .async_write_secret(write_request("APP_KEY", &value, Some(""), None, None))
        .await
        .unwrap();
    assert!(matches!(result, VaultOperationResult::Response(_)));
    let body = sent_body(transport.recorded()[0].body.as_deref().unwrap());
    assert_eq!(body["data"]["description"], "");
}

#[rstest]
#[case::total(SecretTimeout::Total(Duration::from_secs(9)), Duration::from_secs(9))]
#[case::phases(
    SecretTimeout::Phases {
        connect: Some(Duration::from_secs(1)),
        read: Some(Duration::from_secs(2)),
        write: Some(Duration::from_secs(3)),
        pool: Some(Duration::from_secs(4)),
    },
    Duration::from_secs(10)
)]
#[case::empty_phases(
    SecretTimeout::Phases {
        connect: None,
        read: None,
        write: None,
        pool: None,
    },
    Duration::from_secs(600)
)]
#[tokio::test]
async fn a_write_passes_the_resolved_timeout(
    #[case] timeout: SecretTimeout,
    #[case] expected: Duration,
) {
    let (manager, transport) = scripted(
        &token_environment(),
        &[(
            "POST http://127.0.0.1:8200/v1/secret/data/APP_KEY",
            vec![answered(200, WRITE_BODY)],
        )],
    );
    let value = SecretString::from("new-value");
    let result = manager
        .async_write_secret(write_request("APP_KEY", &value, None, None, Some(timeout)))
        .await
        .unwrap();
    assert!(matches!(result, VaultOperationResult::Response(_)));
    assert_eq!(transport.recorded()[0].timeout, expected);
}

#[rstest]
#[case::http_error(answered(400, ""))]
#[case::transport_failure(Reply::Failure)]
#[case::not_json(answered(200, "{"))]
#[case::json_array(answered(200, "[]"))]
#[tokio::test]
async fn a_failed_write_returns_an_error_status(#[case] reply: Reply) {
    let (manager, _transport) = scripted(
        &token_environment(),
        &[(
            "POST http://127.0.0.1:8200/v1/secret/data/APP_KEY",
            vec![reply],
        )],
    );
    let value = SecretString::from("new-value");
    let result = manager
        .async_write_secret(write_request("APP_KEY", &value, None, None, None))
        .await
        .unwrap();
    let VaultOperationResult::Status(status) = result else {
        panic!("a failed write returns a status dict");
    };
    assert_eq!(status.status, OperationStatus::Error);
    assert!(!status.message.contains("new-value"));
}

#[tokio::test]
async fn a_write_of_an_unsafe_name_returns_an_error_status_without_a_request() {
    let (manager, transport) = scripted(&token_environment(), &[]);
    let value = SecretString::from("new-value");
    let result = manager
        .async_write_secret(write_request("../escape", &value, None, None, None))
        .await
        .unwrap();
    let VaultOperationResult::Status(status) = result else {
        panic!("an unsafe name returns a status dict");
    };
    assert_eq!(status.status, OperationStatus::Error);
    assert!(transport.recorded().is_empty());
}

#[tokio::test]
async fn a_write_does_not_refresh_the_read_cache() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[
            (
                READ_URL,
                vec![answered(200, SECRET_BODY), answered(200, OTHER_SECRET_BODY)],
            ),
            (
                "POST http://127.0.0.1:8200/v1/secret/data/APP_KEY",
                vec![answered(200, WRITE_BODY)],
            ),
        ],
    );
    let first = manager
        .async_read_secret("APP_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(first.unwrap().expose_secret(), "sk-from-vault");
    let value = SecretString::from("new-value");
    let _write = manager
        .async_write_secret(write_request("APP_KEY", &value, None, None, None))
        .await
        .unwrap();
    let second = manager
        .async_read_secret("APP_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(second.unwrap().expose_secret(), "sk-from-vault");
    let requests = transport.recorded();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, VaultMethod::Get);
    assert_eq!(requests[1].method, VaultMethod::Post);
}

#[tokio::test]
async fn deleting_a_secret_invalidates_the_cached_body() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[
            (
                READ_URL,
                vec![answered(200, SECRET_BODY), answered(200, OTHER_SECRET_BODY)],
            ),
            (
                "DELETE http://127.0.0.1:8200/v1/secret/data/APP_KEY",
                vec![answered(204, "")],
            ),
        ],
    );
    let first = manager
        .async_read_secret("APP_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(first.unwrap().expose_secret(), "sk-from-vault");
    let deleted = manager
        .async_delete_secret("APP_KEY", delete_request())
        .await
        .unwrap();
    let status = status_of(deleted);
    assert_eq!(status.status, OperationStatus::Success);
    assert_eq!(status.message, "Secret APP_KEY deleted successfully");
    let second = manager
        .async_read_secret("APP_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(second.unwrap().expose_secret(), "other-secret");
    let requests = transport.recorded();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].method, VaultMethod::Delete);
}

#[tokio::test]
async fn a_failed_delete_returns_an_error_status_and_keeps_the_cached_body() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[
            (
                READ_URL,
                vec![answered(200, SECRET_BODY), answered(200, OTHER_SECRET_BODY)],
            ),
            (
                "DELETE http://127.0.0.1:8200/v1/secret/data/APP_KEY",
                vec![answered(500, "")],
            ),
        ],
    );
    let first = manager
        .async_read_secret("APP_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(first.unwrap().expose_secret(), "sk-from-vault");
    let deleted = manager
        .async_delete_secret("APP_KEY", delete_request())
        .await
        .unwrap();
    assert_eq!(status_of(deleted).status, OperationStatus::Error);
    let second = manager
        .async_read_secret("APP_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(second.unwrap().expose_secret(), "sk-from-vault");
    assert_eq!(transport.recorded().len(), 2);
}

#[tokio::test]
async fn rotating_to_a_new_name_writes_verifies_and_deletes_the_old_secret() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[
            (OLD_GET, vec![answered(200, SECRET_BODY)]),
            (NEW_POST, vec![answered(200, WRITE_BODY)]),
            (NEW_GET, vec![answered(200, NEW_SECRET_BODY)]),
            (OLD_DELETE, vec![answered(204, "")]),
        ],
    );
    let value = SecretString::from("new-secret");
    let result = manager
        .async_rotate_secret(rotation("OLD_KEY", "NEW_KEY", &value, None))
        .await
        .unwrap();
    let VaultOperationResult::Response(response) = result else {
        panic!("a successful rotation returns the create response");
    };
    assert_eq!(response.body.expose_secret(), WRITE_BODY);
    let requests = transport.recorded();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].method, VaultMethod::Get);
    assert_eq!(requests[1].method, VaultMethod::Post);
    assert_eq!(requests[2].method, VaultMethod::Get);
    assert_eq!(requests[3].method, VaultMethod::Delete);
    let body = sent_body(requests[1].body.as_deref().unwrap());
    assert_eq!(body["data"]["key"], "new-secret");
    assert_eq!(body["data"]["description"], "Rotated from OLD_KEY");
}

#[tokio::test]
async fn rotating_the_same_name_updates_without_deleting() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[
            (
                NEW_GET,
                vec![answered(200, SECRET_BODY), answered(200, NEW_SECRET_BODY)],
            ),
            (NEW_POST, vec![answered(200, WRITE_BODY)]),
        ],
    );
    let value = SecretString::from("new-secret");
    let result = manager
        .async_rotate_secret(rotation("NEW_KEY", "NEW_KEY", &value, None))
        .await
        .unwrap();
    assert!(matches!(result, VaultOperationResult::Response(_)));
    let requests = transport.recorded();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].method, VaultMethod::Get);
    assert_eq!(requests[1].method, VaultMethod::Post);
    assert_eq!(requests[2].method, VaultMethod::Get);
}

#[tokio::test]
async fn rotating_a_missing_current_secret_returns_an_error() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[(OLD_GET, vec![answered(404, r#"{"errors":[]}"#)])],
    );
    let value = SecretString::from("new-secret");
    let result = manager
        .async_rotate_secret(rotation("OLD_KEY", "NEW_KEY", &value, None))
        .await
        .unwrap();
    let VaultOperationResult::Status(status) = result else {
        panic!("a missing current secret returns a status dict");
    };
    assert_eq!(status.status, OperationStatus::Error);
    assert!(status.message.contains("OLD_KEY"));
    assert_eq!(transport.recorded().len(), 1);
}

#[tokio::test]
async fn rotating_returns_the_write_error_when_the_new_write_fails() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[
            (OLD_GET, vec![answered(200, SECRET_BODY)]),
            (NEW_POST, vec![answered(400, "")]),
        ],
    );
    let value = SecretString::from("new-secret");
    let result = manager
        .async_rotate_secret(rotation("OLD_KEY", "NEW_KEY", &value, None))
        .await
        .unwrap();
    let VaultOperationResult::Status(status) = result else {
        panic!("a failed write returns a status dict");
    };
    assert_eq!(status.status, OperationStatus::Error);
    assert_eq!(transport.recorded().len(), 2);
}

#[tokio::test]
async fn rotating_returns_an_error_when_the_new_value_does_not_match() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[
            (OLD_GET, vec![answered(200, SECRET_BODY)]),
            (NEW_POST, vec![answered(200, WRITE_BODY)]),
            (NEW_GET, vec![answered(200, OTHER_SECRET_BODY)]),
        ],
    );
    let value = SecretString::from("new-secret");
    let result = manager
        .async_rotate_secret(rotation("OLD_KEY", "NEW_KEY", &value, None))
        .await
        .unwrap();
    let VaultOperationResult::Status(status) = result else {
        panic!("a value mismatch returns a status dict");
    };
    assert_eq!(status.status, OperationStatus::Error);
    assert!(!status.message.contains("new-secret"));
    assert!(!status.message.contains("other-secret"));
    assert_eq!(transport.recorded().len(), 3);
}

#[tokio::test]
async fn rotating_fails_when_the_new_secret_cannot_be_verified() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[
            (OLD_GET, vec![answered(200, SECRET_BODY)]),
            (NEW_POST, vec![answered(200, WRITE_BODY)]),
            (NEW_GET, vec![answered(404, "")]),
        ],
    );
    let value = SecretString::from("new-secret");
    let result = manager
        .async_rotate_secret(rotation("OLD_KEY", "NEW_KEY", &value, None))
        .await
        .unwrap();
    let VaultOperationResult::Status(status) = result else {
        panic!("a missing new secret returns a status dict");
    };
    assert_eq!(status.status, OperationStatus::Error);
    assert!(status.message.contains("NEW_KEY"));
    assert_eq!(transport.recorded().len(), 3);
}

#[tokio::test]
async fn a_failed_delete_after_rotation_is_a_warning_and_keeps_the_old_cached_body() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[
            (OLD_GET, vec![answered(200, SECRET_BODY)]),
            (NEW_POST, vec![answered(200, WRITE_BODY)]),
            (NEW_GET, vec![answered(200, NEW_SECRET_BODY)]),
            (OLD_DELETE, vec![answered(500, "")]),
        ],
    );
    let cached = manager
        .async_read_secret("OLD_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(cached.unwrap().expose_secret(), "sk-from-vault");
    let (logs, guard) = capture_logs();
    let value = SecretString::from("new-secret");
    let result = manager
        .async_rotate_secret(rotation("OLD_KEY", "NEW_KEY", &value, None))
        .await
        .unwrap();
    drop(guard);
    assert!(matches!(result, VaultOperationResult::Response(_)));
    assert!(
        logs.text()
            .contains("failed to delete old secret after rotation")
    );
    assert_eq!(transport.recorded().len(), 5);
    let still_cached = manager
        .async_read_secret("OLD_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(still_cached.unwrap().expose_secret(), "sk-from-vault");
    assert_eq!(transport.recorded().len(), 5);
}

#[tokio::test]
async fn a_successful_rotation_clears_the_old_cached_body() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[
            (OLD_GET, vec![answered(200, SECRET_BODY)]),
            (NEW_POST, vec![answered(200, WRITE_BODY)]),
            (NEW_GET, vec![answered(200, NEW_SECRET_BODY)]),
            (OLD_DELETE, vec![answered(204, "")]),
        ],
    );
    let first = manager
        .async_read_secret("OLD_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(first.unwrap().expose_secret(), "sk-from-vault");
    let value = SecretString::from("new-secret");
    let _rotated = manager
        .async_rotate_secret(rotation("OLD_KEY", "NEW_KEY", &value, None))
        .await
        .unwrap();
    let second = manager
        .async_read_secret("OLD_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(second.unwrap().expose_secret(), "sk-from-vault");
    let gets = transport
        .recorded()
        .into_iter()
        .filter(|request| request.method == VaultMethod::Get && request.url == OLD_URL)
        .count();
    assert_eq!(gets, 3);
}

#[tokio::test]
async fn a_rotation_clears_the_new_cached_body() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[
            (OLD_GET, vec![answered(200, SECRET_BODY)]),
            (NEW_POST, vec![answered(200, WRITE_BODY)]),
            (
                NEW_GET,
                vec![
                    answered(200, OTHER_SECRET_BODY),
                    answered(200, NEW_SECRET_BODY),
                    answered(200, NEW_SECRET_BODY),
                ],
            ),
            (OLD_DELETE, vec![answered(204, "")]),
        ],
    );
    let primed = manager
        .async_read_secret("NEW_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(primed.unwrap().expose_secret(), "other-secret");
    let value = SecretString::from("new-secret");
    let _rotated = manager
        .async_rotate_secret(rotation("OLD_KEY", "NEW_KEY", &value, None))
        .await
        .unwrap();
    let after = manager
        .async_read_secret("NEW_KEY", read_options(None, None))
        .await
        .unwrap();
    assert_eq!(after.unwrap().expose_secret(), "new-secret");
    let gets = transport
        .recorded()
        .into_iter()
        .filter(|request| request.method == VaultMethod::Get && request.url == NEW_URL)
        .count();
    assert_eq!(gets, 3);
}

#[tokio::test]
async fn a_rotation_get_timeout_returns_the_fixed_message() {
    let (manager, transport) = scripted(
        &token_environment(),
        &[(
            OLD_GET,
            vec![slow(200, SECRET_BODY, Duration::from_millis(100))],
        )],
    );
    let value = SecretString::from("new-secret");
    let timeout = Some(SecretTimeout::Total(Duration::from_millis(10)));
    let result = manager
        .async_rotate_secret(rotation("OLD_KEY", "NEW_KEY", &value, timeout))
        .await
        .unwrap();
    let VaultOperationResult::Status(status) = result else {
        panic!("a timeout returns a status dict");
    };
    assert_eq!(status.status, OperationStatus::Error);
    assert_eq!(status.message, "Timeout error occurred");
    assert_eq!(transport.recorded()[0].timeout, Duration::from_millis(10));
}

#[tokio::test]
async fn a_write_timeout_returns_its_own_error_status() {
    let (manager, _transport) = scripted(
        &token_environment(),
        &[(
            "POST http://127.0.0.1:8200/v1/secret/data/APP_KEY",
            vec![slow(200, WRITE_BODY, Duration::from_millis(100))],
        )],
    );
    let value = SecretString::from("new-value");
    let timeout = Some(SecretTimeout::Total(Duration::from_millis(10)));
    let result = manager
        .async_write_secret(write_request("APP_KEY", &value, None, None, timeout))
        .await
        .unwrap();
    let VaultOperationResult::Status(status) = result else {
        panic!("a timeout returns a status dict");
    };
    assert_eq!(status.status, OperationStatus::Error);
    assert_eq!(status.message, "the Vault request timed out");
}

#[test]
fn the_read_body_decoder_returns_the_value_at_the_data_key() {
    let parsed = vault_secret_read_body(SECRET_BODY.as_bytes()).unwrap();
    let exposed = |name| parsed.value(name).map(secrecy::ExposeSecret::expose_secret);
    assert_eq!(exposed("key"), Some("sk-from-vault"));
    assert_eq!(exposed("password"), Some("pw-from-vault"));
    assert_eq!(exposed("missing"), None);
}

#[test]
fn a_parsed_read_body_never_prints_the_secret() {
    let parsed = vault_secret_read_body(SECRET_BODY.as_bytes()).unwrap();
    let printed = format!("{parsed:?}");
    assert!(!printed.contains("sk-from-vault"));
    assert!(!printed.contains("pw-from-vault"));
}

#[test]
fn the_read_body_decoder_tolerates_other_values_in_the_data_block() {
    let body = br#"{"data":{"data":{"key":"value","count":2,"nested":{"a":1}}}}"#;
    let parsed = vault_secret_read_body(body).unwrap();
    assert_eq!(
        parsed
            .value("key")
            .map(secrecy::ExposeSecret::expose_secret),
        Some("value")
    );
    assert!(parsed.value("count").is_none());
}

#[rstest]
#[case::not_json("not json")]
#[case::array_document("[]")]
#[case::null_document("null")]
#[case::data_is_not_an_object(r#"{"data":5}"#)]
#[case::inner_data_is_not_an_object(r#"{"data":{"data":[]}}"#)]
fn the_read_body_decoder_refuses_a_broken_document(#[case] body: &str) {
    assert_eq!(
        vault_secret_read_body(body.as_bytes()).unwrap_err(),
        VaultResponseError::InvalidJson
    );
}

#[tokio::test]
async fn an_oversized_write_response_returns_an_error_status() {
    let oversized = format!(
        r#"{{"data":{{"data":{{"key":"{}"}}}}}}"#,
        "x".repeat(MAX_RESPONSE_BYTES)
    );
    let (manager, _transport) = scripted(
        &token_environment(),
        &[(
            "POST http://127.0.0.1:8200/v1/secret/data/APP_KEY",
            vec![answered(200, &oversized)],
        )],
    );
    let value = SecretString::from("new-value");
    let result = manager
        .async_write_secret(write_request("APP_KEY", &value, None, None, None))
        .await
        .unwrap();
    let VaultOperationResult::Status(status) = result else {
        panic!("an oversized response returns a status dict");
    };
    assert_eq!(
        status.message,
        format!("the Vault response is longer than {MAX_RESPONSE_BYTES} bytes")
    );
}

#[test]
fn replays_the_vault_response_corpus_through_the_read_body_decoder() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/vault_response");
    let decoded = fs::read_dir(&corpus)
        .unwrap()
        .map(|entry| vault_secret_read_body(&fs::read(entry.unwrap().path()).unwrap()))
        .collect::<Vec<_>>();
    assert!(decoded.len() >= 6);
}
