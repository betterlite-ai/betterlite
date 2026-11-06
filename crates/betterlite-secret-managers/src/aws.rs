use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use secrecy::SecretString;

pub mod client_init;
pub mod request_signing;

#[derive(Debug)]
pub struct AwsDependencies<Resolver, Transport> {
    pub environment: BTreeMap<String, String>,
    pub transport: Arc<Transport>,
    pub resolver: Resolver,
    pub dns_suffix: fn(&str) -> &'static str,
    pub clock: fn() -> SystemTime,
}

#[derive(Debug)]
pub struct AwsSecretsManager<Resolver, Transport> {
    pub aws_region_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_session_name: Option<String>,
    pub aws_external_id: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>,
    pub replica_regions: Vec<String>,
    pub kms_key_id: Option<String>,
    pub dependencies: AwsDependencies<Resolver, Transport>,
}
