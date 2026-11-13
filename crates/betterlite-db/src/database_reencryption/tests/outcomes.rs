use secrecy::SecretString;

use super::fixtures::{MASTER_KEY, PREVIOUS_KEY};
use crate::database_reencryption::MigrationError;
use crate::database_reencryption::outcome::{
    MigrationFailure, MigrationOutcome, NothingToMigrate, describe_outcome,
};

#[test]
fn advice_salt() {
    assert_eq!(
        describe_outcome(&MigrationOutcome::Nothing(
            NothingToMigrate::SaltKeyEncryptsStoredValues
        )),
        "LITELLM_MIGRATE_FROM_MASTER_KEY is set, but LITELLM_SALT_KEY is what encrypts your stored values, so there is nothing to migrate. You may now delete LITELLM_MIGRATE_FROM_MASTER_KEY."
    );
}

#[test]
fn advice_no_database_conditional() {
    assert_eq!(
        describe_outcome(&MigrationOutcome::Nothing(NothingToMigrate::NoDatabase)),
        "LITELLM_MIGRATE_FROM_MASTER_KEY is set, but no database is connected, so nothing was migrated. If this proxy has no database, you may now delete LITELLM_MIGRATE_FROM_MASTER_KEY."
    );
}

#[test]
fn advice_no_old_values() {
    assert_eq!(
        describe_outcome(&MigrationOutcome::Nothing(
            NothingToMigrate::NothingLeftToMigrate
        )),
        "LITELLM_MIGRATE_FROM_MASTER_KEY is still set, but nothing in the database is left to migrate from that key. You may now delete LITELLM_MIGRATE_FROM_MASTER_KEY."
    );
}

#[test]
fn advice_migrated_complete() {
    assert_eq!(
        describe_outcome(&MigrationOutcome::Migrated {
            migrated: 4,
            remaining: 0
        }),
        "Done re-encrypting 4 stored value(s) with the new master key. You may now delete the LITELLM_MIGRATE_FROM_MASTER_KEY environment variable."
    );
}

#[test]
fn advice_remaining_restart() {
    assert_eq!(
        describe_outcome(&MigrationOutcome::Migrated {
            migrated: 3,
            remaining: 2
        }),
        "Re-encrypted 3 stored value(s), but 2 are still encrypted with the previous key because they changed during the migration. Keep LITELLM_MIGRATE_FROM_MASTER_KEY set and restart the proxy to migrate them."
    );
}

#[test]
fn advice_failed_unreadable() {
    let failure = MigrationFailure::from_cause(
        MigrationError::Reported {
            message: "boom".to_owned(),
        },
        &[],
    );
    assert_eq!(
        describe_outcome(&MigrationOutcome::Failed(failure)),
        "Could not migrate stored values from the LITELLM_MIGRATE_FROM_MASTER_KEY key (ReportedError: boom). Values still encrypted with the previous key cannot be read until the migration succeeds. Keep LITELLM_MIGRATE_FROM_MASTER_KEY set and restart the proxy once the database is reachable."
    );
}

#[test]
fn native_migration_failure_redacts_secrets() {
    let previous = SecretString::from(PREVIOUS_KEY);
    let master = SecretString::from(MASTER_KEY);
    let failure = MigrationFailure::from_cause(
        MigrationError::Reported {
            message: format!("the old {PREVIOUS_KEY} and the new {MASTER_KEY} both leaked"),
        },
        &[&previous, &master],
    );
    let debug = format!("{failure:?}");
    assert!(!debug.contains(PREVIOUS_KEY), "{debug}");
    assert!(!debug.contains(MASTER_KEY), "{debug}");
    let text = describe_outcome(&MigrationOutcome::Failed(failure));
    assert!(!text.contains(PREVIOUS_KEY), "{text}");
    assert!(!text.contains(MASTER_KEY), "{text}");
    assert!(text.contains("[redacted]"), "{text}");
}

#[test]
fn failure_cause_limit_300_characters() {
    let message = "x".repeat(400);
    let truncated: String = format!("ReportedError: {message}")
        .chars()
        .take(300)
        .collect();
    let expected = format!(
        "Could not migrate stored values from the LITELLM_MIGRATE_FROM_MASTER_KEY key ({truncated}). Values still encrypted with the previous key cannot be read until the migration succeeds. Keep LITELLM_MIGRATE_FROM_MASTER_KEY set and restart the proxy once the database is reachable."
    );
    let ascii = MigrationFailure::from_cause(MigrationError::Reported { message }, &[]);
    assert_eq!(describe_outcome(&MigrationOutcome::Failed(ascii)), expected);
    let multibyte = "\u{e9}".repeat(400);
    let limited: String = format!("ReportedError: {multibyte}")
        .chars()
        .take(300)
        .collect();
    assert_eq!(limited.chars().count(), 300);
    let unicode =
        MigrationFailure::from_cause(MigrationError::Reported { message: multibyte }, &[]);
    assert!(
        describe_outcome(&MigrationOutcome::Failed(unicode)).contains(&limited),
        "the bound counts characters, not bytes"
    );
}
