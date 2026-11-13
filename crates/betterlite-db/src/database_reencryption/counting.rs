use betterlite_secrets::secret_codec::decrypt_if_encrypted_with;
use secrecy::SecretString;

use super::MigrationError;
use super::registry::SecretColumn;
use super::storage::SecretStorage;
use super::traversal::replace_ciphertexts;
use crate::encryption_scan::MigrationJson;

pub async fn count_values_encrypted_with<D>(
    database: &D,
    signing_key: &SecretString,
    max_depth: i64,
) -> Result<usize, MigrationError>
where
    D: SecretStorage + Sync,
{
    let mut counted = Vec::new();
    for column in database.secret_columns().await? {
        counted.push(count_column(database, &column, signing_key, max_depth).await?);
    }
    Ok(counted.into_iter().sum())
}

async fn count_column<D>(
    database: &D,
    column: &SecretColumn,
    signing_key: &SecretString,
    max_depth: i64,
) -> Result<usize, MigrationError>
where
    D: SecretStorage + Sync,
{
    let rows = database.rows(column).await?;
    let counted = rows
        .into_iter()
        .map(|row| count_row(row.value, signing_key, max_depth))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(counted.into_iter().sum())
}

pub async fn count_encrypted_or_none<D, C>(
    connect: C,
    signing_key: &SecretString,
    max_depth: i64,
) -> Option<usize>
where
    C: FnOnce() -> Result<D, MigrationError> + Send,
    D: SecretStorage + Send + Sync,
{
    let database = connect().ok()?;
    count_values_encrypted_with(&database, signing_key, max_depth)
        .await
        .ok()
}

fn count_row(
    stored: Option<MigrationJson>,
    signing_key: &SecretString,
    max_depth: i64,
) -> Result<usize, MigrationError> {
    let Some(value) = stored else {
        return Ok(0);
    };
    // LiteLLM keeps a value only when the explicit key decrypts it, and counts leaves, not rows
    let mut replacement = |text: &str| {
        Ok::<_, MigrationError>(
            decrypt_if_encrypted_with(text, signing_key).map(|_| text.to_owned()),
        )
    };
    let (_, count) = replace_ciphertexts(value, &mut replacement, 0, max_depth)?;
    Ok(count)
}
