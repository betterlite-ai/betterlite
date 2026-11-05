use aws_sdk_kms::config::{SharedCredentialsProvider, SharedHttpClient};
use std::{collections::BTreeMap, time::Duration};

use super::{
    AwsKmsError, DecryptedValue,
    initialization::{build_client, validate_environment},
    values::decrypt_with,
};

pub async fn decrypt_env_var(
    environment: &BTreeMap<String, String>,
    transport: SharedHttpClient,
    credentials: Option<SharedCredentialsProvider>,
    operation_timeout: Duration,
) -> Result<BTreeMap<String, DecryptedValue>, AwsKmsError> {
    let region = validate_environment(environment)?;
    let client = build_client(region, transport, credentials, operation_timeout);
    let mut decrypted_values = BTreeMap::new();
    for (key, value) in environment {
        if key
            .to_ascii_lowercase()
            .starts_with("litellm_secret_aws_kms")
            || value.starts_with("aws_kms/")
        {
            let decrypted = decrypt_with(&client, value).await?;
            decrypted_values.insert(remove_secret_prefix(key), decrypted);
        }
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

#[path = "environment/decrypt_env_var/tests.rs"]
mod decrypt_env_var;
