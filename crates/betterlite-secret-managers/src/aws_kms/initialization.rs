use std::sync::Arc;

use aws_config::BehaviorVersion;
use aws_sdk_kms::{Client, config::Region};

use super::{AWS_KMS_CLIENT, AwsKmsError, environment_snapshot};

pub fn validate_environment() -> Result<String, AwsKmsError> {
    let environment = environment_snapshot();
    let region = environment.get("AWS_REGION_NAME").map(String::as_str);
    validate_region(region)
}

fn validate_region(region: Option<&str>) -> Result<String, AwsKmsError> {
    region.map_or(Err(AwsKmsError::MissingRegion), |configured_region| {
        Ok(configured_region.to_owned())
    })
}

pub async fn load_aws_kms(use_aws_kms: Option<bool>) -> Result<(), AwsKmsError> {
    if use_aws_kms != Some(true) {
        return Ok(());
    }
    let region = validate_environment()?;
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new(region))
        .load()
        .await;
    let client = Arc::new(Client::new(&config));
    let mut stored_client = AWS_KMS_CLIENT
        .lock()
        .map_err(|_poisoned| AwsKmsError::ClientState)?;
    if stored_client.is_some() {
        return Err(AwsKmsError::Request(
            "AWS KMS client is already initialized".to_owned(),
        ));
    }
    *stored_client = Some(client);
    drop(stored_client);
    Ok(())
}

mod tests;
