use std::fmt;

use secrecy::{ExposeSecret as _, SecretString};

use super::{MAX_CAUSE_CHARS, MigrationError, PREVIOUS_KEY_VARIABLE, SALT_KEY_ENV_VAR};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NothingToMigrate {
    SaltKeyEncryptsStoredValues,
    NoDatabase,
    NothingLeftToMigrate,
}

pub struct MigrationFailure {
    pub cause: MigrationError,
    pub class: String,
    pub message: String,
}

impl MigrationFailure {
    #[must_use]
    pub fn from_cause(cause: MigrationError, secrets: &[&SecretString]) -> Self {
        let class = error_class(&cause).to_owned();
        let message = redacted(&cause.to_string(), secrets);
        Self {
            cause,
            class,
            message,
        }
    }
}

// the cause can reach operator logs, so it shows the class and the scrubbed text and nothing else
impl fmt::Debug for MigrationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MigrationFailure")
            .field("class", &self.class)
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum MigrationOutcome {
    Nothing(NothingToMigrate),
    Migrated { migrated: usize, remaining: usize },
    Failed(MigrationFailure),
}

#[must_use]
pub fn describe_outcome(outcome: &MigrationOutcome) -> String {
    match outcome {
        MigrationOutcome::Nothing(NothingToMigrate::SaltKeyEncryptsStoredValues) => format!(
            "{PREVIOUS_KEY_VARIABLE} is set, but {SALT_KEY_ENV_VAR} is what encrypts your stored values, so there is nothing to migrate. You may now delete {PREVIOUS_KEY_VARIABLE}."
        ),
        MigrationOutcome::Nothing(NothingToMigrate::NoDatabase) => format!(
            "{PREVIOUS_KEY_VARIABLE} is set, but no database is connected, so nothing was migrated. If this proxy has no database, you may now delete {PREVIOUS_KEY_VARIABLE}."
        ),
        MigrationOutcome::Nothing(NothingToMigrate::NothingLeftToMigrate) => format!(
            "{PREVIOUS_KEY_VARIABLE} is still set, but nothing in the database is left to migrate from that key. You may now delete {PREVIOUS_KEY_VARIABLE}."
        ),
        MigrationOutcome::Migrated {
            migrated,
            remaining: 0,
        } => format!(
            "Done re-encrypting {migrated} stored value(s) with the new master key. You may now delete the {PREVIOUS_KEY_VARIABLE} environment variable."
        ),
        MigrationOutcome::Migrated {
            migrated,
            remaining,
        } => format!(
            "Re-encrypted {migrated} stored value(s), but {remaining} are still encrypted with the previous key because they changed during the migration. Keep {PREVIOUS_KEY_VARIABLE} set and restart the proxy to migrate them."
        ),
        MigrationOutcome::Failed(failure) => {
            let cause: String = format!("{}: {}", failure.class, failure.message)
                .chars()
                .take(MAX_CAUSE_CHARS)
                .collect();
            format!(
                "Could not migrate stored values from the {PREVIOUS_KEY_VARIABLE} key ({cause}). Values still encrypted with the previous key cannot be read until the migration succeeds. Keep {PREVIOUS_KEY_VARIABLE} set and restart the proxy once the database is reachable."
            )
        }
    }
}

const fn error_class(error: &MigrationError) -> &'static str {
    match error {
        MigrationError::Read(_) => "PostgresReadError",
        MigrationError::Write(_) => "PostgresWriteError",
        MigrationError::InvalidRow { .. } => "InvalidStoredRow",
        MigrationError::Decode(_) => "StoredValueDecodeError",
        MigrationError::NotText => "ScalarNotString",
        MigrationError::Encrypt(_) => "SecretEncryptionError",
        MigrationError::Json(_) => "StoredValueJsonError",
        MigrationError::Connect(_) => "ConnectError",
        MigrationError::Logger(_) => "LoggerError",
        MigrationError::Unrebuildable => "RebuildError",
    }
}

fn redacted(message: &str, secrets: &[&SecretString]) -> String {
    let mut redacted = message.to_owned();
    for secret in secrets {
        let value = secret.expose_secret();
        if !value.is_empty() {
            redacted = redacted.replace(value, "[redacted]");
        }
    }
    redacted
}
