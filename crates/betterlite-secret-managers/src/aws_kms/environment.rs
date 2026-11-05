use aws_sdk_kms::config::{SharedCredentialsProvider, SharedHttpClient};
use regex::Regex;
use std::{collections::BTreeMap, time::Duration};

use super::{AwsKmsError, DecryptedValue, initialization::build_client, values::decrypt_with};

pub async fn decrypt_env_var(
    environment: &BTreeMap<String, String>,
    transport: SharedHttpClient,
    credentials: Option<SharedCredentialsProvider>,
    operation_timeout: Duration,
) -> Result<BTreeMap<String, DecryptedValue>, AwsKmsError> {
    let client = build_client(environment, transport, credentials, operation_timeout)?;
    let secret_marker = Regex::new("(?i)litellm_secret_aws_kms_")?;
    let mut decrypted_values = BTreeMap::new();
    for (key, value) in environment {
        if key.to_lowercase().starts_with("litellm_secret_aws_kms") || value.starts_with("aws_kms/")
        {
            let decrypted = decrypt_with(&client, value).await?;
            decrypted_values.insert(secret_marker.replace_all(key, "").into_owned(), decrypted);
        }
    }
    Ok(decrypted_values)
}

#[path = "environment/decrypt_env_var/tests.rs"]
mod decrypt_env_var;
