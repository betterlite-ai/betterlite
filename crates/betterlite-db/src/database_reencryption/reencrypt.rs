use betterlite_secrets::secret_codec::{StoredValue, decrypt_if_encrypted_with};
use secrecy::{ExposeSecret as _, SecretString};

use super::MigrationError;
use super::registry::SecretColumn;
use super::runner::MigrationPlan;
use super::storage::{SecretRow, SecretStorage};
use super::traversal::{as_sql_parameter, replace_ciphertexts};

pub async fn reencrypt_stored_values<D>(
    database: &D,
    plan: &MigrationPlan<'_>,
) -> Result<usize, MigrationError>
where
    D: SecretStorage + Sync,
{
    let mut total: usize = 0;
    for column in database.secret_columns().await? {
        for row in database.rows(&column).await? {
            let reencrypted = reencrypt_row(database, &column, row, plan).await?;
            total = total.saturating_add(reencrypted);
        }
    }
    Ok(total)
}

async fn reencrypt_row<D>(
    database: &D,
    column: &SecretColumn,
    row: SecretRow,
    plan: &MigrationPlan<'_>,
) -> Result<usize, MigrationError>
where
    D: SecretStorage + Sync,
{
    let Some(value) = row.value else {
        return Ok(0);
    };
    let old = as_sql_parameter(&value, column.is_json)?;
    let mut replacement = |text: &str| reencrypted(text, plan);
    let (migrated, count) = replace_ciphertexts(value, &mut replacement, 0, plan.max_depth)?;
    if count == 0 {
        return Ok(0);
    }
    let new = as_sql_parameter(&migrated, column.is_json)?;
    let updated = database
        .replace(column, &old, &new, &row.primary_key)
        .await?;
    Ok(if updated { count } else { 0 })
}

fn reencrypted(value: &str, plan: &MigrationPlan<'_>) -> Result<Option<String>, MigrationError> {
    let Some(plaintext) = decrypt_if_encrypted_with(value, plan.previous_master_key) else {
        return Ok(None);
    };
    let sealed = plan
        .cipher
        .encrypt_value(StoredValue::<()>::Text(plaintext), Some(plan.master_key))
        .map_err(MigrationError::Encrypt)?;
    let StoredValue::Text(ciphertext) = sealed else {
        return Err(MigrationError::NotText);
    };
    Ok(Some(ciphertext.expose_secret().to_owned()))
}
