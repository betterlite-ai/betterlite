use std::fmt::Debug;

use async_trait::async_trait;
use secrecy::SecretString;

use super::verdict::{BootVerdict, Refusal, StoredSecretsMigration};

#[async_trait]
pub trait StoredSecretCounter: Debug + Send + Sync {
    async fn count_values_encrypted_with(&self, signing_key: &SecretString) -> Option<usize>;
}

pub async fn with_stored_secrets_counted(
    verdict: BootVerdict,
    counter: &dyn StoredSecretCounter,
) -> BootVerdict {
    match verdict {
        BootVerdict::Refused(refusal) => BootVerdict::Refused(counted(refusal, counter).await),
        untouched @ (BootVerdict::Safe | BootVerdict::Allowed { .. }) => untouched,
    }
}

async fn counted(mut refusal: Refusal, counter: &dyn StoredSecretCounter) -> Refusal {
    let Some(migration) = refusal.migration.take() else {
        return refusal;
    };
    let found = counter
        .count_values_encrypted_with(&migration.from_master_key)
        .await;
    refusal.migration = match found {
        Some(0) => None,
        Some(total) => Some(StoredSecretsMigration {
            from_master_key: migration.from_master_key,
            encrypted_value_count: Some(total),
        }),
        None => Some(migration),
    };
    refusal
}
