use std::convert::Infallible;

use betterlite_secret_managers::vault_secret_operations::result::VaultOperationResult;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

pub(super) fn infallible<T>(outcome: Result<T, Infallible>) -> T {
    match outcome {
        Ok(value) => value,
        Err(never) => match never {},
    }
}

pub(super) fn read_outcome(value: Option<SecretString>) -> Value {
    value.map_or_else(
        || json!({"value": Value::Null}),
        |secret| json!({"value": secret.expose_secret()}),
    )
}

pub(super) fn operation_outcome(result: &VaultOperationResult) -> Value {
    match result {
        VaultOperationResult::Response(response) => {
            let body =
                serde_json::from_str::<Value>(response.body.expose_secret()).unwrap_or(Value::Null);
            json!({"body": body})
        }
        VaultOperationResult::Status(status) => json!({"status": status.status}),
    }
}
