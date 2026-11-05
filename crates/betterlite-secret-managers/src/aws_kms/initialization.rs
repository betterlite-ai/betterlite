use std::{collections::BTreeMap, sync::Arc, time::Duration};

use aws_sdk_kms::{
    Client,
    config::{
        BehaviorVersion, Builder, Region, SharedCredentialsProvider, SharedHttpClient,
        timeout::TimeoutConfig,
    },
};

use super::{AWS_KMS_CLIENT, AwsKmsError};

pub fn validate_environment(environment: &BTreeMap<String, String>) -> Result<String, AwsKmsError> {
    environment
        .get("AWS_REGION_NAME")
        .cloned()
        .ok_or(AwsKmsError::MissingRegion)
}

pub fn load_aws_kms(
    use_aws_kms: Option<bool>,
    environment: &BTreeMap<String, String>,
    transport: SharedHttpClient,
    credentials: Option<SharedCredentialsProvider>,
    operation_timeout: Duration,
) -> Result<(), AwsKmsError> {
    if use_aws_kms != Some(true) {
        return Ok(());
    }
    let region = validate_environment(environment)?;
    let client = build_client(region, transport, credentials, operation_timeout);
    store_client(client)
}

pub(crate) fn build_client(
    region: String,
    transport: SharedHttpClient,
    credentials: Option<SharedCredentialsProvider>,
    operation_timeout: Duration,
) -> Arc<Client> {
    let mut config = Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .http_client(transport)
        .region(Region::new(region))
        .timeout_config(
            TimeoutConfig::builder()
                .operation_timeout(operation_timeout)
                .build(),
        );
    config.set_credentials_provider(credentials);
    Arc::new(Client::from_conf(config.build()))
}

fn store_client(client: Arc<Client>) -> Result<(), AwsKmsError> {
    let mut stored_client = AWS_KMS_CLIENT
        .lock()
        .map_err(|_poisoned| AwsKmsError::ClientState)?;
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
