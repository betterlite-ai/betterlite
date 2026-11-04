use std::collections::BTreeMap;

use super::{AwsKmsError, DecryptedValue, environment_snapshot, values::decrypt_value};

pub async fn decrypt_env_var() -> Result<BTreeMap<String, DecryptedValue>, AwsKmsError> {
    let mut decrypted_values = BTreeMap::new();
    let encrypted_values: Vec<(String, String)> = environment_snapshot()
        .into_iter()
        .filter(|(key, value)| {
            key.to_ascii_lowercase()
                .starts_with("litellm_secret_aws_kms_")
                || value.starts_with("aws_kms/")
        })
        .collect();
    for (key, value) in encrypted_values {
        let decrypted = decrypt_value(&value).await?;
        decrypted_values.insert(remove_secret_prefix(&key), decrypted);
    }
    Ok(decrypted_values)
}

fn remove_secret_prefix(key: &str) -> String {
    let prefix = "litellm_secret_aws_kms_";
    match key.get(..prefix.len()) {
        Some(start) if start.eq_ignore_ascii_case(prefix) => key
            .get(prefix.len()..)
            .map_or_else(|| key.to_owned(), str::to_owned),
        _ => key.to_owned(),
    }
}

mod tests;
