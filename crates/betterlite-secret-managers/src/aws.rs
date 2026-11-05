use std::collections::BTreeMap;

use secrecy::SecretString;

pub mod client_init;
pub mod request_signing;

#[derive(Debug)]
pub struct AwsDependencies<Resolver> {
    pub environment: BTreeMap<String, String>,
    pub http_client: reqwest::Client,
    pub resolver: Resolver,
    pub dns_suffix: fn(&str) -> &'static str,
}

#[derive(Debug)]
pub struct AwsSecretsManager<Resolver> {
    pub aws_region_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_session_name: Option<String>,
    pub aws_external_id: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>,
    pub replica_regions: Vec<String>,
    pub kms_key_id: Option<String>,
    pub dependencies: AwsDependencies<Resolver>,
}
