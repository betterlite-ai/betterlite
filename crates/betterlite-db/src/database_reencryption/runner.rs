use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use super::counting::{count_values_encrypted_with, reencrypt_stored_values};
use super::outcome::{MigrationFailure, MigrationOutcome, NothingToMigrate, describe_outcome};
use super::storage::SecretStorage;
use super::{MigrationError, PREVIOUS_KEY_VARIABLE};
use crate::stored_cipher::DbCipher;

pub struct MigrationRequest<'keys> {
    pub previous_master_key: Option<&'keys SecretString>,
    pub master_key: Option<&'keys SecretString>,
    pub salt_key_is_set: bool,
    pub cipher: &'keys DbCipher,
    pub max_depth: i64,
}

pub struct MigrationPlan<'keys> {
    pub previous_master_key: &'keys SecretString,
    pub master_key: &'keys SecretString,
    pub salt_key_is_set: bool,
    pub cipher: &'keys DbCipher,
    pub max_depth: i64,
}

pub async fn migrate_if_requested<'database, D, Q, L, T>(
    request: &MigrationRequest<'_>,
    connected_database: Q,
    log: L,
    raise_unless_tolerated: T,
) -> Result<Option<MigrationOutcome>, MigrationFailure>
where
    D: SecretStorage + Sync + 'database,
    Q: FnOnce() -> Result<Option<&'database D>, MigrationFailure> + Send,
    L: Fn(&str) -> Result<(), MigrationError> + Send + Sync,
    T: Fn(&MigrationFailure) -> Result<(), MigrationFailure> + Send,
{
    let (Some(previous_master_key), Some(master_key)) =
        (request.previous_master_key, request.master_key)
    else {
        return Ok(None);
    };
    let plan = MigrationPlan {
        previous_master_key,
        master_key,
        salt_key_is_set: request.salt_key_is_set,
        cipher: request.cipher,
        max_depth: request.max_depth,
    };
    let database = connected_database()?;
    let outcome = migrate_from_previous_key(&plan, database, &log).await?;
    if let MigrationOutcome::Failed(failure) = &outcome {
        raise_unless_tolerated(failure)?;
    }
    Ok(Some(outcome))
}

pub async fn migrate_from_previous_key<D, L>(
    plan: &MigrationPlan<'_>,
    database: Option<&D>,
    log: L,
) -> Result<MigrationOutcome, MigrationFailure>
where
    D: SecretStorage + Sync,
    L: Fn(&str) -> Result<(), MigrationError> + Send + Sync,
{
    let outcome = migrate_or_failure(plan, database, &log).await;
    let secrets = [plan.previous_master_key, plan.master_key];
    log(&describe_outcome(&outcome))
        .map_err(|cause| MigrationFailure::from_cause(cause, &secrets))?;
    Ok(outcome)
}

async fn migrate_or_failure<D, L>(
    plan: &MigrationPlan<'_>,
    database: Option<&D>,
    log: &L,
) -> MigrationOutcome
where
    D: SecretStorage + Sync,
    L: Fn(&str) -> Result<(), MigrationError> + Sync,
{
    let secrets = [plan.previous_master_key, plan.master_key];
    match migrate(plan, database, log).await {
        Ok(outcome) => outcome,
        Err(cause) => MigrationOutcome::Failed(MigrationFailure::from_cause(cause, &secrets)),
    }
}

async fn migrate<D, L>(
    plan: &MigrationPlan<'_>,
    database: Option<&D>,
    log: &L,
) -> Result<MigrationOutcome, MigrationError>
where
    D: SecretStorage + Sync,
    L: Fn(&str) -> Result<(), MigrationError> + Sync,
{
    if plan.salt_key_is_set {
        return Ok(MigrationOutcome::Nothing(
            NothingToMigrate::SaltKeyEncryptsStoredValues,
        ));
    }
    let Some(connected) = database else {
        return Ok(MigrationOutcome::Nothing(NothingToMigrate::NoDatabase));
    };
    if same_key(plan) {
        return Ok(MigrationOutcome::Nothing(
            NothingToMigrate::NothingLeftToMigrate,
        ));
    }
    let found =
        count_values_encrypted_with(connected, plan.previous_master_key, plan.max_depth).await?;
    if found == 0 {
        return Ok(MigrationOutcome::Nothing(
            NothingToMigrate::NothingLeftToMigrate,
        ));
    }
    log(&format!(
        "Re-encrypting {found} stored value(s) from the {PREVIOUS_KEY_VARIABLE} key to the new master key."
    ))?;
    reencrypt_and_recount(plan, connected).await
}

async fn reencrypt_and_recount<D>(
    plan: &MigrationPlan<'_>,
    database: &D,
) -> Result<MigrationOutcome, MigrationError>
where
    D: SecretStorage + Sync,
{
    let migrated = reencrypt_stored_values(database, plan).await?;
    let remaining =
        count_values_encrypted_with(database, plan.previous_master_key, plan.max_depth).await?;
    if migrated == 0 && remaining == 0 {
        return Ok(MigrationOutcome::Nothing(
            NothingToMigrate::NothingLeftToMigrate,
        ));
    }
    Ok(MigrationOutcome::Migrated {
        migrated,
        remaining,
    })
}

fn same_key(plan: &MigrationPlan<'_>) -> bool {
    bool::from(
        plan.previous_master_key
            .expose_secret()
            .as_bytes()
            .ct_eq(plan.master_key.expose_secret().as_bytes()),
    )
}
