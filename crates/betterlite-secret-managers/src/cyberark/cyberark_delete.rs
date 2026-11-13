use super::CyberArkSecretManager;
use super::operation::{OperationStatus, SecretOperation};

mod tests;

pub(super) fn delete<T>(manager: &CyberArkSecretManager<T>, secret_name: &str) -> SecretOperation {
    tracing::warn!(
        "CyberArk Conjur does not support direct secret deletion. Secrets must be removed through policy updates."
    );
    manager.values.delete(secret_name);
    SecretOperation {
        status: OperationStatus::NotSupported,
        message: "CyberArk Conjur does not support direct secret deletion. Use policy updates to remove variables."
            .to_owned(),
    }
}
