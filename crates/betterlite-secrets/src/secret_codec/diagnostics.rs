use crate::error::SecretError;

const SALT_DOCS: &str = "https://docs.litellm.ai/docs/proxy/prod#5-set-litellm-salt-key";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Debug,
    Error,
}

#[derive(Debug)]
pub enum SecretEvent<'event> {
    NotAString {
        value_type: &'event str,
    },
    UnableToDecrypt {
        field: &'event str,
    },
    DecryptFailed {
        field: &'event str,
        error: &'event SecretError,
    },
}

impl SecretEvent<'_> {
    // values never reach a message: only the field name, the Rust type and the error kind do
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NotAString { value_type } => {
                format!(
                    "Invalid value type passed to encrypt_value: {value_type}; value must be a string"
                )
            }
            Self::UnableToDecrypt { field } => {
                format!("Unable to decrypt value for key: {field}, returning None")
            }
            Self::DecryptFailed { field, error } => format!(
                "Error decrypting value for key: {field}, did your master_key/salt key change recently? Error: {error}. Set a permanent salt key: {SALT_DOCS}"
            ),
        }
    }
}

pub trait SecretDiagnostics {
    fn record(&self, level: Level, event: &SecretEvent<'_>);
}

pub struct TracingDiagnostics;

impl SecretDiagnostics for TracingDiagnostics {
    fn record(&self, level: Level, event: &SecretEvent<'_>) {
        let message = event.message();
        match level {
            Level::Debug => tracing::debug!(%message),
            Level::Error => tracing::error!(%message),
        }
    }
}
