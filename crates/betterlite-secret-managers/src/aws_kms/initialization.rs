use std::sync::Arc;

use aws_sdk_kms::{
    Client,
    config::{BehaviorVersion, Builder, Region, SharedCredentialsProvider, SharedHttpClient},
};

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

pub fn load_aws_kms(
    use_aws_kms: Option<bool>,
    transport: SharedHttpClient,
    credentials: Option<SharedCredentialsProvider>,
) -> Result<(), AwsKmsError> {
    if use_aws_kms != Some(true) {
        return Ok(());
    }
    let region = validate_environment()?;
    let client = build_client(region, transport, credentials);
    store_client(client)
}

fn build_client(
    region: String,
    transport: SharedHttpClient,
    credentials: Option<SharedCredentialsProvider>,
) -> Arc<Client> {
    let mut config = Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .http_client(transport)
        .region(Region::new(region));
    config.set_credentials_provider(credentials);
    Arc::new(Client::from_conf(config.build()))
}

fn store_client(client: Arc<Client>) -> Result<(), AwsKmsError> {
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

#[path = "initialization/client_storage/tests.rs"]
mod client_storage;
#[path = "initialization/load_aws_kms/tests.rs"]
mod load_aws_kms;
#[path = "initialization/validate_environment/tests.rs"]
mod validate_environment;
