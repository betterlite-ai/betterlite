use std::collections::BTreeMap;
use std::error::Error;

use secrecy::SecretString;
use tracing::warn;

use crate::aws::{AwsDependencies, AwsSecretsManager};
use crate::secret_types::KeyManagementSystem;

#[derive(Debug, Clone)]
pub struct AwsKeyManagementSettings {
    pub aws_region_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_session_name: Option<String>,
    pub aws_external_id: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>,
    pub replica_regions: Vec<String>,
    pub kms_key_id: Option<String>,
}

pub trait AwsSecretManagerRuntime<Resolver> {
    type Error: Error + Send + Sync + 'static;

    fn set_secret_manager_client(
        &mut self,
        client: AwsSecretsManager<Resolver>,
    ) -> Result<(), Self::Error>;
    fn set_key_management_system(&mut self, system: KeyManagementSystem)
    -> Result<(), Self::Error>;
}

pub fn validate_environment(environment: &BTreeMap<String, String>) {
    let has_region = ["AWS_REGION_NAME", "AWS_REGION", "AWS_DEFAULT_REGION"]
        .iter()
        .any(|name| environment.contains_key(*name));
    if !has_region {
        warn!(
            "No AWS region found in environment. Ensure aws_region_name is set in key_management_settings or AWS_REGION_NAME/AWS_REGION/AWS_DEFAULT_REGION is set in environment."
        );
    }
}

pub fn load_aws_secret_manager<Resolver, Runtime>(
    use_aws_secret_manager: Option<bool>,
    key_management_settings: Option<&AwsKeyManagementSettings>,
    dependencies: AwsDependencies<Resolver>,
    runtime: &mut Runtime,
) -> Result<(), Runtime::Error>
where
    Runtime: AwsSecretManagerRuntime<Resolver>,
{
    if use_aws_secret_manager != Some(true) {
        return Ok(());
    }
    validate_environment(&dependencies.environment);
    let client = match key_management_settings {
        Some(settings) => AwsSecretsManager {
            aws_region_name: settings.aws_region_name.clone(),
            aws_role_name: settings.aws_role_name.clone(),
            aws_session_name: settings.aws_session_name.clone(),
            aws_external_id: settings.aws_external_id.clone(),
            aws_profile_name: settings.aws_profile_name.clone(),
            aws_web_identity_token: settings.aws_web_identity_token.clone(),
            aws_sts_endpoint: settings.aws_sts_endpoint.clone(),
            replica_regions: settings.replica_regions.clone(),
            kms_key_id: settings.kms_key_id.clone(),
            dependencies,
        },
        None => AwsSecretsManager {
            aws_region_name: None,
            aws_role_name: None,
            aws_session_name: None,
            aws_external_id: None,
            aws_profile_name: None,
            aws_web_identity_token: None,
            aws_sts_endpoint: None,
            replica_regions: Vec::new(),
            kms_key_id: None,
            dependencies,
        },
    };
    runtime.set_secret_manager_client(client)?;
    runtime.set_key_management_system(KeyManagementSystem::AwsSecretManager)
}

#[path = "client_init/aws_client_init/tests.rs"]
mod aws_client_init;
