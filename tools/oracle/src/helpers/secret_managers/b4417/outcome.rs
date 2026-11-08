use std::convert::Infallible;

use betterlite_secret_managers::cyberark::operation::OperationStatus;
use betterlite_secret_managers::secret_contracts::RotationError;
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

pub(super) fn rotation_outcome(
    outcome: Result<VaultOperationResult, RotationError<Infallible>>,
) -> Value {
    match outcome {
        Ok(result) => operation_outcome(&result),
        Err(RotationError::Operation { source }) => match source {},
        Err(
            RotationError::CurrentSecretNotFound
            | RotationError::NewSecretNotVerified
            | RotationError::SameName,
        ) => json!({"status": "error"}),
    }
}

pub(super) fn operation_outcome(result: &VaultOperationResult) -> Value {
    match result {
        VaultOperationResult::Response(response) => {
            let body =
                serde_json::from_str::<Value>(response.body.expose_secret()).unwrap_or(Value::Null);
            json!({"body": body})
        }
        VaultOperationResult::Status(_status) => json!({"status": "error"}),
    }
}

pub(super) fn deletion_outcome(result: &VaultOperationResult) -> Value {
    match result {
        VaultOperationResult::Status(status) if status.status == OperationStatus::Success => {
            json!({"status": status.status, "message": status.message})
        }
        VaultOperationResult::Status(_) | VaultOperationResult::Response(_) => {
            json!({"status": "error"})
        }
    }
}
